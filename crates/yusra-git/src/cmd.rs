//! Menjalankan `git` dengan lingkungan yang terisolasi dari proses pemanggil.

use std::{
    ffi::OsStr,
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
    thread,
};

use yusra_core::YusraError;

/// Variabel lingkungan Git yang bisa membelokkan perintah ke repo, index,
/// atau object store lain jika daemon dijalankan dari dalam hook Git.
const INHERITED_GIT_ENV: [&str; 12] = [
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_INDEX_FILE",
    "GIT_OBJECT_DIRECTORY",
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_COMMON_DIR",
    "GIT_NAMESPACE",
    "GIT_CEILING_DIRECTORIES",
    "GIT_DISCOVERY_ACROSS_FILESYSTEM",
    "GIT_LITERAL_PATHSPECS",
    "GIT_GLOB_PATHSPECS",
    "GIT_ICASE_PATHSPECS",
];

/// Identitas untuk commit snapshot, agar tidak bergantung pada
/// `user.name`/`user.email` pengguna dan tidak tercampur dengan commit mereka.
const IDENTITY: [(&str, &str); 4] = [
    ("GIT_AUTHOR_NAME", "yusra"),
    ("GIT_AUTHOR_EMAIL", "yusra@localhost"),
    ("GIT_COMMITTER_NAME", "yusra"),
    ("GIT_COMMITTER_EMAIL", "yusra@localhost"),
];

/// Batas stderr yang ikut ke pesan error.
const MAX_STDERR_CHARS: usize = 500;

/// Satu pemanggilan `git -C <dir> ...`.
pub(crate) struct Git {
    command: Command,
    display: String,
    stdin: Option<Vec<u8>>,
}

impl Git {
    pub(crate) fn new(dir: &Path) -> Self {
        let mut command = Command::new("git");
        for var in INHERITED_GIT_ENV {
            command.env_remove(var);
        }
        command
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GIT_OPTIONAL_LOCKS", "0")
            .env("LC_ALL", "C")
            .envs(IDENTITY)
            .arg("-C")
            .arg(dir)
            // Hook dan fsmonitor pengguna tidak boleh jalan atas nama yusra.
            .args([
                "-c",
                "core.hooksPath=/dev/null",
                "-c",
                "core.fsmonitor=false",
            ])
            .stdin(Stdio::null());
        Self {
            command,
            display: "git".to_owned(),
            stdin: None,
        }
    }

    pub(crate) fn arg(mut self, arg: impl AsRef<OsStr>) -> Self {
        let arg = arg.as_ref();
        if self.display.len() < 200 {
            self.display.push(' ');
            self.display.push_str(&arg.to_string_lossy());
        }
        self.command.arg(arg);
        self
    }

    pub(crate) fn args<I, S>(self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        args.into_iter().fold(self, Self::arg)
    }

    pub(crate) fn env(mut self, key: &str, value: impl AsRef<OsStr>) -> Self {
        self.command.env(key, value);
        self
    }

    pub(crate) fn stdin(mut self, bytes: &[u8]) -> Self {
        self.stdin = Some(bytes.to_vec());
        self
    }

    /// Jalankan dan kembalikan output apa pun status keluarnya.
    pub(crate) fn output(mut self) -> Result<Output, YusraError> {
        let display = self.display;
        let spawn_error = |e| YusraError::internal_with(format!("gagal menjalankan {display}"), e);
        let Some(input) = self.stdin else {
            return self.command.output().map_err(spawn_error);
        };
        let mut child = self
            .command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(spawn_error)?;
        let Some(mut pipe) = child.stdin.take() else {
            return Err(YusraError::internal(format!(
                "stdin {display} tidak tersedia"
            )));
        };
        // Ditulis dari thread lain agar stdout yang penuh tidak membuat
        // kedua proses saling menunggu.
        let writer = thread::spawn(move || pipe.write_all(&input));
        let out = child.wait_with_output().map_err(spawn_error)?;
        match writer.join() {
            Ok(Ok(())) => Ok(out),
            Ok(Err(e)) => Err(YusraError::internal_with(
                format!("gagal menulis stdin {display}"),
                e,
            )),
            Err(_) => Err(YusraError::internal(format!(
                "thread stdin {display} panik"
            ))),
        }
    }

    /// Jalankan dan kembalikan stdout; status gagal menjadi error internal.
    pub(crate) fn run(self) -> Result<Vec<u8>, YusraError> {
        let display = self.display.clone();
        let out = self.output()?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(failed(&display, &out))
        }
    }

    /// Seperti [`Git::run`], stdout dirapikan menjadi satu baris teks.
    pub(crate) fn run_line(self) -> Result<String, YusraError> {
        let display = self.display.clone();
        let stdout = self.run()?;
        String::from_utf8(stdout)
            .map(|s| s.trim_end().to_owned())
            .map_err(|e| YusraError::internal_with(format!("output {display} bukan UTF-8"), e))
    }
}

fn failed(display: &str, out: &Output) -> YusraError {
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stderr: String = stderr.trim().chars().take(MAX_STDERR_CHARS).collect();
    YusraError::internal(format!("{display} gagal ({}): {stderr}", out.status))
}
