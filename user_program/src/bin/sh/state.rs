//! Shell state: variables, positional parameters, last exit status, options.

use alloc::{
    collections::BTreeMap,
    string::{String, ToString},
    vec::Vec,
};

use user_lib::env;

/// One shell variable: value plus whether it is exported into the child env.
#[derive(Clone)]
pub struct Var {
    pub value: String,
    pub exported: bool,
    pub readonly: bool,
}

/// Whole-process shell state.
///
/// Lives for the duration of one shell process. Functions, command groups,
/// and `.`-sourced scripts all share the same state by passing `&mut State`
/// down through the executor.
pub struct State {
    /// Named variables. The `PATH`, `IFS`, `HOME`, `PWD`, `OLDPWD`, `PS1`,
    /// `PS2` entries are seeded from the inherited environment at startup.
    vars: BTreeMap<String, Var>,
    /// Shell function definitions (`name() { … }`).
    funcs: BTreeMap<String, crate::ast::Cmd>,
    /// Positional parameters: `$1`, `$2`, … (does NOT include `$0`).
    params: Vec<String>,
    /// `$0` — the shell's argv[0] or the currently sourced script.
    pub arg0: String,
    /// `$?` — last command exit status.
    pub last_status: i32,
    /// `$!` — pid of the most recent background job.
    pub last_bg_pid: u32,
    /// Whether `-e` (errexit) is in effect: exit on a failed command.
    pub errexit: bool,
    /// Whether `-x` (xtrace) is in effect: trace expansions to stderr.
    pub xtrace: bool,
    /// Whether `-u` (nounset) is in effect: error on undefined var expansion.
    pub nounset: bool,
}

impl State {
    /// Builds initial state from the host process's argv + envp.
    pub fn from_env(arg0: String, params: Vec<String>) -> Self {
        let mut vars = BTreeMap::new();
        for (key, value) in env::vars() {
            vars.insert(key, Var {
                value,
                exported: true,
                readonly: false,
            });
        }
        // Provide sane defaults for missing entries so unquoted expansion never
        // surprises with an empty PATH or IFS.
        if !vars.contains_key("PATH") {
            vars.insert("PATH".to_string(), Var {
                value: "/bin:/usr/bin:/usr/local/bin".to_string(),
                exported: true,
                readonly: false,
            });
        }
        if !vars.contains_key("IFS") {
            vars.insert("IFS".to_string(), Var {
                value: " \t\n".to_string(),
                exported: false,
                readonly: false,
            });
        }
        if !vars.contains_key("PS1") {
            vars.insert("PS1".to_string(), Var {
                value: "$ ".to_string(),
                exported: false,
                readonly: false,
            });
        }
        if !vars.contains_key("PS2") {
            vars.insert("PS2".to_string(), Var {
                value: "> ".to_string(),
                exported: false,
                readonly: false,
            });
        }

        Self {
            vars,
            funcs: BTreeMap::new(),
            params,
            arg0,
            last_status: 0,
            last_bg_pid: 0,
            errexit: false,
            xtrace: false,
            nounset: false,
        }
    }

    /// Returns the value of `name`, or `None` if unset.
    pub fn get(&self, name: &str) -> Option<&str> {
        self.vars.get(name).map(|v| v.value.as_str())
    }

    /// Sets `name` to `value`, preserving the existing `exported` flag.
    pub fn set(&mut self, name: &str, value: String) {
        if let Some(v) = self.vars.get_mut(name) {
            if v.readonly {
                return;
            }
            v.value = value;
        } else {
            self.vars.insert(name.to_string(), Var {
                value,
                exported: false,
                readonly: false,
            });
        }
    }

    /// Marks `name` as exported and optionally updates the value.
    pub fn export(&mut self, name: &str, value: Option<String>) {
        let entry = self.vars.entry(name.to_string()).or_insert(Var {
            value: String::new(),
            exported: false,
            readonly: false,
        });
        if let Some(v) = value {
            if !entry.readonly {
                entry.value = v;
            }
        }
        entry.exported = true;
    }

    /// Removes a variable.
    pub fn unset(&mut self, name: &str) {
        if let Some(v) = self.vars.get(name) {
            if v.readonly {
                return;
            }
        }
        self.vars.remove(name);
    }

    /// Returns `(KEY, VALUE)` pairs for every exported variable.
    pub fn exported_pairs(&self) -> Vec<(String, String)> {
        self.vars
            .iter()
            .filter(|(_, v)| v.exported)
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect()
    }

    /// Returns `(KEY, VALUE)` pairs for every variable (exported or not).
    pub fn all_pairs(&self) -> Vec<(String, String)> {
        self.vars
            .iter()
            .map(|(k, v)| (k.clone(), v.value.clone()))
            .collect()
    }

    /// Returns the positional parameter at `index` (1-based), or empty.
    pub fn positional(&self, index: usize) -> &str {
        if index == 0 {
            self.arg0.as_str()
        } else {
            self.params.get(index - 1).map(String::as_str).unwrap_or("")
        }
    }

    /// Returns the number of positional parameters (`$#`).
    pub fn positional_count(&self) -> usize {
        self.params.len()
    }

    /// Returns a slice over `$@` / `$*`.
    pub fn positionals(&self) -> &[String] {
        &self.params
    }

    /// Replaces the positional-parameter list (used by `set --` and function
    /// invocation).
    pub fn set_positionals(&mut self, new: Vec<String>) {
        self.params = new;
    }

    /// Takes ownership of the positional-parameter list, replacing it with
    /// `new` and returning the previous value. Used by `.` / `source` and
    /// by function calls to temporarily swap in script-local parameters.
    pub fn replace_positionals(&mut self, new: Vec<String>) -> Vec<String> {
        core::mem::replace(&mut self.params, new)
    }

    /// `shift n` — drops the first `n` positional parameters.
    /// Returns `false` if `n` is larger than `$#`.
    pub fn shift(&mut self, n: usize) -> bool {
        if n > self.params.len() {
            return false;
        }
        self.params.drain(..n);
        true
    }

    /// Registers a function definition (overwrites any previous one).
    pub fn define_function(&mut self, name: String, body: crate::ast::Cmd) {
        self.funcs.insert(name, body);
    }

    /// Looks up a function body by name.
    pub fn function(&self, name: &str) -> Option<&crate::ast::Cmd> {
        self.funcs.get(name)
    }

    /// Iterates over `(name, body)` pairs for every defined function.
    /// Used by [`clone_for_subshell`](crate::exec) to propagate functions
    /// into a forked subshell.
    pub fn all_functions(&self) -> impl Iterator<Item = (&str, &crate::ast::Cmd)> {
        self.funcs.iter().map(|(k, v)| (k.as_str(), v))
    }

    /// Removes a function definition.
    pub fn undefine_function(&mut self, name: &str) {
        self.funcs.remove(name);
    }
}
