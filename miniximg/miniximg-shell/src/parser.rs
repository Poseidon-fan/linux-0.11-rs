//! Single-line tokenizer.
//!
//! Just enough of "shell" to be ergonomic: words separated by whitespace,
//! single and double quoting, backslash escapes. No expansions, no
//! pipelines, no operators — every other special character is delivered
//! to the command unchanged. The `@` host-path prefix and glob
//! metacharacters get to the command verbatim and are interpreted later.

use anyhow::{Result, bail};

/// Splits `line` into argv-style tokens, respecting quoting.
pub fn tokenize(line: &str) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let bytes = line.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' => {
                if in_token {
                    out.push(std::mem::take(&mut cur));
                    in_token = false;
                }
                i += 1;
            }
            b'\'' => {
                in_token = true;
                i += 1;
                let start = i;
                while i < bytes.len() && bytes[i] != b'\'' {
                    i += 1;
                }
                if i >= bytes.len() {
                    bail!("unterminated single quote");
                }
                cur.push_str(&line[start..i]);
                i += 1;
            }
            b'"' => {
                in_token = true;
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    if bytes[i] == b'\\' && i + 1 < bytes.len() {
                        // Inside `"..."`, backslash escapes only \, $, ", `, and newline.
                        let next = bytes[i + 1];
                        if matches!(next, b'\\' | b'"' | b'$' | b'`' | b'\n') {
                            cur.push(next as char);
                            i += 2;
                            continue;
                        }
                    }
                    cur.push(bytes[i] as char);
                    i += 1;
                }
                if i >= bytes.len() {
                    bail!("unterminated double quote");
                }
                i += 1;
            }
            b'\\' => {
                if i + 1 >= bytes.len() {
                    bail!("trailing backslash");
                }
                in_token = true;
                cur.push(bytes[i + 1] as char);
                i += 2;
            }
            _ => {
                in_token = true;
                cur.push(c as char);
                i += 1;
            }
        }
    }
    if in_token {
        out.push(cur);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_words() {
        assert_eq!(tokenize("ls /etc").unwrap(), vec!["ls", "/etc"]);
        assert_eq!(tokenize("  ls   /etc  ").unwrap(), vec!["ls", "/etc"]);
    }

    #[test]
    fn quoting() {
        assert_eq!(
            tokenize("echo 'hello world'").unwrap(),
            vec!["echo", "hello world"]
        );
        assert_eq!(
            tokenize(r#"echo "a b"  "c""#).unwrap(),
            vec!["echo", "a b", "c"]
        );
        assert_eq!(tokenize(r#"echo a\ b"#).unwrap(), vec!["echo", "a b"]);
    }

    #[test]
    fn at_prefix_preserved() {
        assert_eq!(
            tokenize("cp @./src /dst").unwrap(),
            vec!["cp", "@./src", "/dst"]
        );
    }

    #[test]
    fn unterminated() {
        assert!(tokenize("echo 'oops").is_err());
        assert!(tokenize("echo \"oops").is_err());
        assert!(tokenize("echo \\").is_err());
    }
}
