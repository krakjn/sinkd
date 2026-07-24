//! Compose scenario event definitions (phase B). Shell executes these via docker compose exec.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpKind {
    Remove {
        path: &'static str,
    },
    Write {
        path: &'static str,
        content: &'static str,
    },
    Rename {
        from: &'static str,
        to: &'static str,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Op {
    pub service: &'static str,
    pub kind: OpKind,
}

/// Second wave of client-side mutations after checkpoint A.
pub fn phase_b() -> Vec<Op> {
    vec![
        Op {
            service: "client-alice",
            kind: OpKind::Remove {
                path: "/watch_alice/file_alice_renamed.txt",
            },
        },
        Op {
            service: "client-alice",
            kind: OpKind::Write {
                path: "/watch_alice/extra_alice.txt",
                content: "alice extra",
            },
        },
        Op {
            service: "client-bob",
            kind: OpKind::Rename {
                from: "/watch_bob/file_bob_3.txt",
                to: "/watch_bob/file_bob_3_renamed.txt",
            },
        },
    ]
}

/// Tab-separated lines for `compose.sh`: `service<TAB>op<TAB>arg...`
pub fn phase_b_script_lines() -> Vec<String> {
    phase_b()
        .into_iter()
        .map(|op| match op.kind {
            OpKind::Remove { path } => format!("{}\trm\t{path}", op.service),
            OpKind::Write { path, content } => {
                format!("{}\twrite\t{path}\t{content}", op.service)
            }
            OpKind::Rename { from, to } => format!("{}\tmv\t{from}\t{to}", op.service),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phase_b_has_three_operations() {
        assert_eq!(phase_b().len(), 3);
    }
}
