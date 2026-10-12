//! Uji ujung ke ujung: biner `yusrad` sebagai server MCP lewat stdio.

use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::mpsc::{Receiver, channel},
    thread,
    time::Duration,
};

use serde_json::{Value, json};

const TIMEOUT: Duration = Duration::from_secs(30);

fn yusrad() -> Command {
    Command::new(env!("CARGO_BIN_EXE_yusrad"))
}

fn write(root: &Path, rel: &str, content: &str) {
    let full = root.join(rel);
    if let Some(parent) = full.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        panic!("{e}");
    }
    if let Err(e) = fs::write(full, content) {
        panic!("{e}");
    }
}

struct Server {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Server {
    fn start(root: &Path) -> Self {
        let mut child = match yusrad()
            .args(["up", "--stdio", "--root"])
            .arg(root)
            .env("YUSRA_LOG", "warn")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
        {
            Ok(c) => c,
            Err(e) => panic!("{e}"),
        };
        let stdin = child.stdin.take();
        let Some(stdout) = child.stdout.take() else {
            panic!("stdout tidak tersedia");
        };
        let (tx, lines) = channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            stdin,
            lines,
        }
    }

    fn send(&mut self, msg: &Value) {
        let Some(stdin) = self.stdin.as_mut() else {
            panic!("stdin sudah ditutup");
        };
        if let Err(e) = writeln!(stdin, "{msg}").and_then(|()| stdin.flush()) {
            panic!("{e}");
        }
    }

    /// Kirim request dan tunggu respons dengan `id` yang sama.
    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}));
        loop {
            let line = match self.lines.recv_timeout(TIMEOUT) {
                Ok(l) => l,
                Err(e) => panic!("tidak ada respons untuk {method}: {e}"),
            };
            let msg: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => panic!("stdout bukan JSON-RPC ({e}): {line}"),
            };
            if msg["id"] == json!(id) {
                return msg;
            }
        }
    }

    fn call(&mut self, id: u64, tool: &str, args: Value) -> Value {
        let resp = self.request(id, "tools/call", json!({"name": tool, "arguments": args}));
        resp["result"].clone()
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn up_requires_init() {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    let out = match yusrad()
        .args(["up", "--stdio", "--root"])
        .arg(dir.path())
        .stdin(Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(e) => panic!("{e}"),
    };
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("yusrad init"));
    assert!(out.stdout.is_empty(), "stdout harus bersih");
}

#[test]
fn stdio_session() {
    let dir = match tempfile::tempdir() {
        Ok(d) => d,
        Err(e) => panic!("{e}"),
    };
    let root = dir.path();
    write(
        root,
        "src/lib.rs",
        "pub fn hello_world() -> u32 {\n    42\n}\n",
    );
    write(
        root,
        "src/main.rs",
        "fn main() {\n    lib::hello_world();\n}\n",
    );
    write(root, ".env", "SECRET=1\n");

    let init = match yusrad().args(["init", "--root"]).arg(root).output() {
        Ok(o) => o,
        Err(e) => panic!("{e}"),
    };
    assert!(
        init.status.success(),
        "{}",
        String::from_utf8_lossy(&init.stderr)
    );
    assert!(root.join(".yusra").is_dir());

    let mut server = Server::start(root);
    let init = server.request(
        1,
        "initialize",
        json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {},
            "clientInfo": {"name": "uji", "version": "0"}
        }),
    );
    assert_eq!(init["result"]["serverInfo"]["name"], "yusrad", "{init}");
    server.send(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));

    let list = server.request(2, "tools/list", json!({}));
    let tools = list["result"]["tools"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(tools.len(), 5, "{list}");
    for t in &tools {
        assert_eq!(t["annotations"]["readOnlyHint"], true, "{t}");
        assert_eq!(
            t["inputSchema"]["additionalProperties"], false,
            "request {} harus menolak field asing",
            t["name"]
        );
    }

    let read = server.call(3, "read_file", json!({"path": "src/lib.rs"}));
    assert_eq!(read["isError"], false, "{read}");
    assert_eq!(
        read["structuredContent"]["content"],
        "pub fn hello_world() -> u32 {\n    42\n}\n"
    );

    let denied = server.call(4, "read_file", json!({"path": ".env"}));
    assert_eq!(denied["isError"], true, "{denied}");
    assert_eq!(denied["structuredContent"]["code"], "path_denied");

    let traversal = server.call(5, "read_file", json!({"path": "../etc/passwd"}));
    assert_eq!(traversal["structuredContent"]["code"], "invalid_path");

    let found = server.call(6, "search_symbols", json!({"query": "hello"}));
    assert_eq!(
        found["structuredContent"]["symbols"][0]["symbol_path"], "hello_world",
        "{found}"
    );

    let map = server.call(7, "get_project_map", json!({}));
    let text = map["structuredContent"]["map"].as_str().unwrap_or_default();
    assert!(text.contains("src/lib.rs:"), "{map}");

    let est = server.call(
        8,
        "estimate_tokens",
        json!({"text": "hello world", "model_family": "openai"}),
    );
    assert_eq!(est["structuredContent"]["tokens"], 2, "{est}");

    // Perubahan di disk masuk ke index lewat watcher.
    write(root, "src/extra.rs", "pub fn freshly_added() {}\n");
    let mut seen = false;
    for attempt in 0..50 {
        let r = server.call(100 + attempt, "search_symbols", json!({"query": "freshly"}));
        if r["structuredContent"]["symbols"]
            .as_array()
            .is_some_and(|a| !a.is_empty())
        {
            seen = true;
            break;
        }
        thread::sleep(Duration::from_millis(100));
    }
    assert!(seen, "watcher tidak meng-index file baru");

    // Menutup stdin mengakhiri proses dengan normal.
    drop(server.stdin.take());
    let status = match server.child.wait() {
        Ok(s) => s,
        Err(e) => panic!("{e}"),
    };
    assert!(status.success());
}
