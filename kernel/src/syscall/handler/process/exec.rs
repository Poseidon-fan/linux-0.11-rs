use alloc::string::String;
use core::arch::asm;
use core::mem;

use linkme::distributed_slice;

#[allow(unused_imports)]
use crate::syscall::SYSCALL_TABLE;

use crate::{
    define_syscall_handler,
    fs::{
        BLOCK_SIZE,
        layout::{InodeModeFlags, InodeType},
        minix::Inode,
        path,
    },
    mm::{
        address::LinAddr,
        frame::{self, PAGE_SIZE, PhysFrame},
        page,
        space::{MemorySpace, TASK_LINEAR_SIZE},
    },
    segment::uaccess,
    signal::NSIG,
    syscall::{EACCES, ENOENT, ENOEXEC, ENOMEM, context::SyscallContext},
    task::{self, task_struct::TASK_OPEN_FILES_LIMIT},
};

const MAX_ARG_PAGES: usize = 32;
const ZMAGIC: u32 = 0o413;

/// a.out executable header (demand-paged format).
#[repr(C)]
#[derive(Clone, Copy)]
struct AoutHeader {
    a_magic: u32,
    a_text: u32,
    a_data: u32,
    a_bss: u32,
    a_syms: u32,
    a_entry: u32,
    a_trsize: u32,
    a_drsize: u32,
}

impl AoutHeader {
    fn from_block(block: &[u8]) -> Option<Self> {
        if block.len() < mem::size_of::<Self>() {
            return None;
        }
        // Safety: AoutHeader is repr(C) with no padding requirements beyond u32.
        Some(unsafe { core::ptr::read_unaligned(block.as_ptr() as *const Self) })
    }

    fn validate(&self, file_size: u32) -> Result<(), u32> {
        if self.a_magic != ZMAGIC {
            return Err(ENOEXEC);
        }
        if self.a_trsize != 0 || self.a_drsize != 0 {
            return Err(ENOEXEC);
        }
        if (self.a_text as u64) + (self.a_data as u64) + (self.a_bss as u64) > 0x3000000 {
            return Err(ENOEXEC);
        }
        let header_plus_payload = (BLOCK_SIZE as u64)
            + (self.a_text as u64)
            + (self.a_data as u64)
            + (self.a_syms as u64);
        if (file_size as u64) < header_plus_payload {
            return Err(ENOEXEC);
        }
        if BLOCK_SIZE != 1024 {
            return Err(ENOEXEC);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Argument page management (RAII)
// ---------------------------------------------------------------------------

/// Collects argv/envp strings into kernel-owned physical pages.
///
/// Pages are allocated lazily from the top of the argument area downward.
/// On drop, any allocated pages are freed.
struct ArgumentPages {
    pages: [Option<PhysFrame>; MAX_ARG_PAGES],
    /// Write cursor, counts down from `MAX_ARG_PAGES * PAGE_SIZE`.
    p: usize,
}

impl ArgumentPages {
    fn new() -> Self {
        Self {
            pages: [const { None }; MAX_ARG_PAGES],
            p: MAX_ARG_PAGES * PAGE_SIZE - 4,
        }
    }

    /// Ensure the page for byte offset `off` is allocated; returns a raw
    /// pointer into the page at that offset.
    fn ensure_page(&mut self, off: usize) -> Result<*mut u8, u32> {
        let page_idx = off / PAGE_SIZE;
        if self.pages[page_idx].is_none() {
            self.pages[page_idx] = Some(frame::alloc().ok_or(ENOMEM)?);
        }
        let frame = self.pages[page_idx].as_ref().unwrap();
        let phys = frame.ppn.addr();
        let page_off = off % PAGE_SIZE;
        Ok((phys.as_u32() + page_off as u32) as *mut u8)
    }

    /// Write one byte at the current cursor and advance downward.
    fn push_byte(&mut self, byte: u8) -> Result<(), u32> {
        if self.p == 0 {
            return Err(ENOMEM);
        }
        self.p -= 1;
        let ptr = self.ensure_page(self.p)?;
        unsafe { *ptr = byte };
        Ok(())
    }

    /// Copy one NUL-terminated string (including the terminator) from user
    /// space into the argument pages.
    fn copy_one_user_string(&mut self, user_ptr: u32) -> Result<(), u32> {
        let len = user_strlen(user_ptr);
        self.push_byte(0)?;
        for i in (0..len).rev() {
            let b = uaccess::read_u8(unsafe { (user_ptr as *const u8).add(i) });
            self.push_byte(b)?;
        }
        Ok(())
    }

    /// Copy `argc` user-space strings whose pointers live at `argv_ptr`.
    /// Strings are copied in reverse order so that argv[0] ends up at the
    /// lowest address (matching the original semantics).
    fn copy_user_strings(&mut self, argv_ptr: *const u32, argc: usize) -> Result<(), u32> {
        for i in (0..argc).rev() {
            let str_ptr = uaccess::read_u32(unsafe { argv_ptr.add(i) });
            if str_ptr == 0 {
                panic!("copy_user_strings: NULL pointer in argv at index {}", i);
            }
            self.copy_one_user_string(str_ptr)?;
        }
        Ok(())
    }

    /// Copy one kernel-space byte slice (with appended NUL) into the argument
    /// pages. Used for #! interpreter name / argument.
    fn copy_kernel_string(&mut self, s: &[u8]) -> Result<(), u32> {
        self.push_byte(0)?;
        for &b in s.iter().rev() {
            self.push_byte(b)?;
        }
        Ok(())
    }

    /// Transfer ownership of all allocated pages into `space`, mapping them
    /// at the top of the data segment.
    ///
    /// Pages are mapped at linear addresses
    /// `[base + data_limit - MAX_ARG_PAGES * PAGE_SIZE, base + data_limit)`.
    fn install_into(&mut self, space: &mut MemorySpace, segment_base: u32, data_limit: u32) {
        let mut lin_addr = segment_base + data_limit;
        for frame_slot in self.pages.iter_mut().rev() {
            lin_addr -= PAGE_SIZE as u32;
            if let Some(frame) = frame_slot.take() {
                let lin_page = LinAddr(lin_addr).floor();
                if space.map_page(lin_page, frame).is_err() {
                    panic!("failed to map argument page");
                }
            }
        }
        page::invalidate_tlb();
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Count NUL-terminated user pointer array length (via FS segment reads).
fn count_user_ptrs(argv: *const u32) -> usize {
    if argv.is_null() {
        return 0;
    }
    let mut n = 0;
    loop {
        let ptr = uaccess::read_u32(unsafe { argv.add(n) });
        if ptr == 0 {
            break;
        }
        n += 1;
    }
    n
}

/// Measure a user-space NUL-terminated string's length (excluding NUL).
fn user_strlen(addr: u32) -> usize {
    let mut len = 0usize;
    loop {
        let b = uaccess::read_u8(unsafe { (addr as *const u8).add(len) });
        if b == 0 {
            break;
        }
        len += 1;
    }
    len
}

/// Check that `inode` is a regular file with execute permission for the
/// current task. Returns `(effective_uid, effective_gid)` that the new
/// process image should run with (accounting for set-uid / set-gid bits).
fn check_exec_permission(inode: &Inode) -> Result<(u16, u16), u32> {
    let inner = inode.inner.lock();
    let disk = &inner.disk_inode;

    if disk.mode.file_type() != InodeType::Regular {
        return Err(EACCES);
    }

    let flags = disk.mode.flags();
    let (euid, egid) = task::current_task()
        .pcb
        .inner
        .exclusive(|t| (t.identity.euid, t.identity.egid));

    let e_uid = if flags.contains(InodeModeFlags::SET_USER_ID) {
        disk.user_id
    } else {
        euid
    };
    let e_gid = if flags.contains(InodeModeFlags::SET_GROUP_ID) {
        disk.group_id as u16
    } else {
        egid
    };

    let mut mode = disk.mode.0;
    if euid == disk.user_id {
        mode >>= 6;
    } else if egid == disk.group_id as u16 {
        mode >>= 3;
    }

    if (mode & 1) == 0 && !((disk.mode.0 & 0o111) != 0 && task::is_super()) {
        return Err(EACCES);
    }

    Ok((e_uid, e_gid))
}

/// Parse a `#!` shebang line from the first block of a script file.
///
/// Returns `(interpreter_path, interpreter_basename, optional_argument)`.
fn parse_shebang(block: &[u8]) -> Result<(String, String, Option<String>), u32> {
    let end = block
        .iter()
        .position(|&b| b == b'\n' || b == 0)
        .unwrap_or(block.len().min(1024));
    let line = core::str::from_utf8(&block[2..end]).map_err(|_| ENOEXEC)?;
    let line = line.trim();
    if line.is_empty() {
        return Err(ENOEXEC);
    }

    let (interp, i_arg) = match line.find([' ', '\t']) {
        Some(pos) => {
            let arg = line[pos..].trim_start();
            (&line[..pos], if arg.is_empty() { None } else { Some(arg) })
        }
        None => (line, None),
    };

    let i_name = interp.rsplit('/').next().unwrap_or(interp);

    Ok((
        String::from(interp),
        String::from(i_name),
        i_arg.map(String::from),
    ))
}

/// Build the `argc / argv[] / envp[]` pointer tables on the user stack.
///
/// Writes through the FS segment (which must already point to the new data
/// segment). Returns the new user stack pointer.
fn create_user_tables(p: u32, argc: usize, envc: usize) -> u32 {
    let mut sp = p & 0xFFFF_FFFC;

    // Reserve space for envp[] array (envc + 1 NULL terminator).
    sp -= ((envc + 1) * 4) as u32;
    let envp_base = sp;
    // Reserve space for argv[] array (argc + 1 NULL terminator).
    sp -= ((argc + 1) * 4) as u32;
    let argv_base = sp;

    // Push (envp_ptr, argv_ptr, argc) — the three arguments to main().
    sp -= 4;
    uaccess::write_u32(envp_base, sp as *mut u32);
    sp -= 4;
    uaccess::write_u32(argv_base, sp as *mut u32);
    sp -= 4;
    uaccess::write_u32(argc as u32, sp as *mut u32);

    let mut scan = p;
    scan = fill_ptr_table(scan, argv_base, argc);
    scan = fill_ptr_table(scan, envp_base, envc);
    let _ = scan;

    sp
}

/// Fill a NUL-terminated pointer table (argv or envp) on the user stack.
///
/// Scans forward from `scan` through NUL-terminated strings, writing each
/// address into consecutive slots at `base`. Appends a NULL terminator.
/// Returns the updated scan position.
fn fill_ptr_table(mut scan: u32, base: u32, count: usize) -> u32 {
    for i in 0..count {
        uaccess::write_u32(scan, unsafe { (base as *mut u32).add(i) });
        while uaccess::read_u8(scan as *const u8) != 0 {
            scan += 1;
        }
        scan += 1;
    }
    uaccess::write_u32(0, unsafe { (base as *mut u32).add(count) });
    scan
}

/// Reload the FS segment register so it picks up the updated LDT data
/// segment descriptor.
#[inline]
fn reload_fs_segment() {
    unsafe {
        asm!(
            "pushl $0x17",
            "pop %fs",
            options(att_syntax, nomem, nostack),
        );
    }
}

// ---------------------------------------------------------------------------
// sys_execve
// ---------------------------------------------------------------------------

define_syscall_handler!(
    user_lib::NR_EXECVE = 11,
    fn sys_execve(ctx: &mut SyscallContext) -> Result<u32, u32> {
        let (filename_ptr, argv_ptr, envp_ptr) = ctx.args();
        let filename = uaccess::read_string(filename_ptr as *const u8, 256);

        let argc = count_user_ptrs(argv_ptr as *const u32);
        let envc = count_user_ptrs(envp_ptr as *const u32);

        let mut arg_pages = ArgumentPages::new();
        let mut final_argc = argc;
        let mut sh_bang = false;

        let mut inode = path::resolve_path(&filename).ok_or(ENOENT)?;

        // Resolve the executable — loops at most once for #! scripts.
        let (header, e_uid, e_gid) = loop {
            let (eu, eg) = check_exec_permission(&inode)?;

            let mut first_block = [0u8; BLOCK_SIZE];
            inode.read_at(0, &mut first_block).map_err(|_| EACCES)?;

            if !sh_bang && first_block[0] == b'#' && first_block[1] == b'!' {
                let (interp_path, interp_name, interp_arg) = parse_shebang(&first_block)?;

                // Copy envp and original argv[1:] from user space.
                arg_pages.copy_user_strings(envp_ptr as *const u32, envc)?;
                if argc > 1 {
                    arg_pages
                        .copy_user_strings(unsafe { (argv_ptr as *const u32).add(1) }, argc - 1)?;
                }
                final_argc = argc.saturating_sub(1);

                // Script filename.
                arg_pages.copy_kernel_string(filename.as_bytes())?;
                final_argc += 1;

                // Optional interpreter argument.
                if let Some(ref arg) = interp_arg {
                    arg_pages.copy_kernel_string(arg.as_bytes())?;
                    final_argc += 1;
                }

                // Interpreter basename as new argv[0].
                arg_pages.copy_kernel_string(interp_name.as_bytes())?;
                final_argc += 1;

                sh_bang = true;
                inode = path::resolve_path(&interp_path).ok_or(ENOENT)?;
                continue;
            }

            let hdr = AoutHeader::from_block(&first_block).ok_or(ENOEXEC)?;
            let file_size = inode.inner.lock().disk_inode.size;
            hdr.validate(file_size)?;

            break (hdr, eu, eg);
        };

        // For normal (non-script) binaries, copy argv and envp now.
        if !sh_bang {
            arg_pages.copy_user_strings(envp_ptr as *const u32, envc)?;
            arg_pages.copy_user_strings(argv_ptr as *const u32, argc)?;
        }

        if arg_pages.p == 0 {
            return Err(ENOMEM);
        }

        // ===== POINT OF NO RETURN =====

        let current = task::current_task();
        let slot = current.pcb.slot;

        let sp = current.pcb.inner.exclusive(|inner| {
            // Replace executable inode.
            inner.fs.executable_inode = Some(inode);

            // Reset all signal handlers to default.
            for sa in &mut inner.signal_info.sigaction[..NSIG] {
                sa.sa_handler = 0;
            }

            // Close file descriptors marked close-on-exec.
            let close_mask = inner.fs.close_on_exec;
            for i in 0..TASK_OPEN_FILES_LIMIT {
                if (close_mask >> i) & 1 == 1 {
                    inner.fs.open_files[i] = None;
                }
            }
            inner.fs.close_on_exec = 0;

            // Release old memory space (page tables + data frames).
            inner.memory_space = None;

            // Create fresh memory space.
            let mut new_space = MemorySpace::new(slot);

            // Configure LDT — code segment covers text pages, data covers 64 MB.
            let code_limit = (header.a_text + PAGE_SIZE as u32 - 1) & !0xFFF;
            inner.ldt.set_code_limit(code_limit >> 12);
            inner.ldt.set_data_limit(TASK_LINEAR_SIZE >> 12);

            reload_fs_segment();

            // Map argument pages at the top of the data segment.
            let seg_base = inner.ldt.data_segment().base();
            arg_pages.install_into(&mut new_space, seg_base, TASK_LINEAR_SIZE);

            inner.memory_space = Some(new_space);

            let p_adj =
                arg_pages.p as u32 + TASK_LINEAR_SIZE - (MAX_ARG_PAGES as u32 * PAGE_SIZE as u32);
            let sp = create_user_tables(p_adj, final_argc, envc);

            inner.mem_layout.end_code = header.a_text;
            inner.mem_layout.end_data = header.a_text + header.a_data;
            inner.mem_layout.brk = header.a_text + header.a_data + header.a_bss;
            inner.mem_layout.start_stack = sp & 0xFFFFF000;

            inner.identity.euid = e_uid;
            inner.identity.egid = e_gid;

            sp
        });

        // Zero the partial page at the end of text+data (BSS alignment).
        let mut i = header.a_text + header.a_data;
        while i & 0xFFF != 0 {
            uaccess::write_u8(0, i as *mut u8);
            i += 1;
        }

        // Redirect iret to the new program's entry point.
        ctx.eip = header.a_entry;
        ctx.user_esp = sp;

        Ok(0)
    }
);
