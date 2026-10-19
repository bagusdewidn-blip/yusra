use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use rmcp::{
    Json, ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{CallToolResult, Implementation, ServerCapabilities, ServerConfig},
    tool, tool_handler, tool_router,
};
use serde::Serialize;
use serde_json::{Value, json};
use yusra_core::YusraError;
use yusra_store::AuditEntry;

use crate::{
    project::{Project, now_ms},
    schema::{
        EstimateTokensRequest, EstimateTokensResponse, GetProjectMapRequest, GetProjectMapResponse,
        ReadFileRequest, ReadFileResponse, ReadSymbolRequest, ReadSymbolResponse,
        SearchSymbolsRequest, SearchSymbolsResponse,
    },
};

const INSTRUCTIONS: &str = "yusra memberi akses baca ke repo lokal. Mulai dengan \
get_project_map untuk gambaran proyek, search_symbols untuk mencari definisi, lalu \
read_symbol atau read_file untuk isi. Semua path relatif terhadap root proyek dengan \
pemisah '/'.";

type ToolResult<T> = Result<Json<T>, CallToolResult>;

#[derive(Clone)]
pub struct YusraServer {
    project: Arc<Project>,
    /// `client_id` di audit log. Untuk stdio nilainya tetap, karena tidak
    /// ada autentikasi.
    client_id: Arc<str>,
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl YusraServer {
    pub fn new(project: Arc<Project>, client_id: &str) -> Self {
        Self {
            project,
            client_id: client_id.into(),
            tool_router: Self::tool_router(),
        }
    }

    /// Peta proyek ringkas: definisi terpenting per file (signature dan
    /// nomor baris), diperingkat dengan PageRank atas graf rujukan dan
    /// dipotong sesuai anggaran token.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn get_project_map(
        &self,
        params: Parameters<GetProjectMapRequest>,
    ) -> ToolResult<GetProjectMapResponse> {
        let req = params.0;
        let args = audit_args(&req);
        self.run("get_project_map", args, move |p| p.project_map(&req))
            .await
    }

    /// Cari simbol (fungsi, struct, class, method, dan seterusnya) yang
    /// namanya memuat `query`.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn search_symbols(
        &self,
        params: Parameters<SearchSymbolsRequest>,
    ) -> ToolResult<SearchSymbolsResponse> {
        let req = params.0;
        let args = audit_args(&req);
        self.run("search_symbols", args, move |p| p.search_symbols(&req))
            .await
    }

    /// Baca isi file, dipotong per baris jika melebihi `max_tokens`.
    /// `file_hash` selalu untuk seluruh file.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn read_file(&self, params: Parameters<ReadFileRequest>) -> ToolResult<ReadFileResponse> {
        let req = params.0;
        let args = audit_args(&req);
        self.run("read_file", args, move |p| p.read_file(&req))
            .await
    }

    /// Baca satu simbol beserta doc comment yang menempel, dari isi file
    /// saat ini.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn read_symbol(
        &self,
        params: Parameters<ReadSymbolRequest>,
    ) -> ToolResult<ReadSymbolResponse> {
        let req = params.0;
        let args = audit_args(&req);
        self.run("read_symbol", args, move |p| p.read_symbol(&req))
            .await
    }

    /// Hitung token untuk teks dan/atau file. Hasil eksak hanya untuk
    /// `openai`; keluarga lain berupa perkiraan.
    #[tool(annotations(read_only_hint = true, open_world_hint = false))]
    async fn estimate_tokens(
        &self,
        params: Parameters<EstimateTokensRequest>,
    ) -> ToolResult<EstimateTokensResponse> {
        let req = params.0;
        // Isi teks tidak boleh masuk audit log; cukup panjangnya.
        let args = json!({
            "text_len": req.text.as_ref().map(String::len),
            "paths": req.paths,
            "model_family": req.model_family,
        });
        self.run("estimate_tokens", args, move |p| p.estimate_tokens(&req))
            .await
    }
}

impl YusraServer {
    /// Tunggu index awal, jalankan `op` di thread blocking, tulis satu baris
    /// audit, dan ubah error menjadi hasil tool `isError: true`.
    async fn run<T, F>(&self, tool: &'static str, args: Value, op: F) -> ToolResult<T>
    where
        T: Send + 'static,
        F: FnOnce(&Project) -> Result<T, YusraError> + Send + 'static,
    {
        let project = Arc::clone(&self.project);
        let client_id = Arc::clone(&self.client_id);
        let joined = tokio::task::spawn_blocking(move || {
            let out = project.wait_ready().and_then(|()| op(&project));
            let outcome = match &out {
                Ok(_) => "ok",
                Err(e) => e.code().as_str(),
            };
            audit(&project, &client_id, tool, &args, outcome);
            out
        })
        .await;
        match joined {
            Ok(Ok(v)) => Ok(Json(v)),
            Ok(Err(e)) => Err(tool_error(tool, &e)),
            Err(join) => {
                let e = YusraError::internal_with("handler panic", join);
                Err(tool_error(tool, &e))
            }
        }
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for YusraServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("yusrad", env!("CARGO_PKG_VERSION")))
            .with_instructions(INSTRUCTIONS)
    }
}

/// `outcome` adalah `ok` atau kode error.
fn audit(project: &Project, client_id: &str, tool: &str, args: &Value, outcome: &str) {
    let args_json = args.to_string();
    let entry = AuditEntry {
        ts: now_ms(),
        client_id: Some(client_id),
        session_id: args.get("session_id").and_then(Value::as_str),
        tool,
        args_json: &args_json,
        outcome,
    };
    if let Err(e) = project.store().append_audit(&entry) {
        tracing::error!(error = %e, tool, "gagal menulis audit log");
    }
}

fn audit_args(req: &impl Serialize) -> Value {
    serde_json::to_value(req).unwrap_or(Value::Null)
}

/// Body error tool: `{ code, message, data }`. Detail error internal hanya
/// masuk log; agen menerima `trace_id` untuk mencocokkannya.
fn tool_error(tool: &str, e: &YusraError) -> CallToolResult {
    let data = match e {
        YusraError::InvalidPath { path, .. } | YusraError::PathDenied { path } => {
            json!({ "path": path })
        }
        YusraError::NotFound { kind, id } => json!({ "kind": kind, "id": id }),
        YusraError::StaleBase {
            path,
            symbol_path,
            expected,
            actual,
        } => json!({
            "path": path,
            "symbol_path": symbol_path,
            "expected": expected.to_hex(),
            "actual": actual.map(|h| h.to_hex()),
        }),
        YusraError::Locked {
            path,
            symbol_path,
            expires_at,
        } => json!({ "path": path, "symbol_path": symbol_path, "expires_at": expires_at }),
        YusraError::ParseError { path } | YusraError::PatchRejected { path, .. } => {
            json!({ "path": path })
        }
        YusraError::SessionClosed { session, status } => {
            json!({ "session_id": session, "status": status.as_str() })
        }
        YusraError::Internal { .. } => {
            let trace_id = trace_id();
            tracing::error!(tool, %trace_id, error = %e, source = ?std::error::Error::source(e), "tool gagal");
            json!({ "trace_id": trace_id })
        }
    };
    let message = match e {
        YusraError::Internal { .. } => "kesalahan internal; lihat log daemon".to_owned(),
        _ => e.to_string(),
    };
    CallToolResult::structured_error(json!({
        "code": e.code().as_str(),
        "message": message,
        "data": data,
    }))
}

/// ID unik per proses untuk mencocokkan error di respons dengan log.
fn trace_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("{:x}-{n:x}", now_ms())
}

/// Layani satu klien lewat stdin/stdout sampai stdin ditutup.
pub async fn serve_stdio(server: YusraServer) -> Result<(), YusraError> {
    let running = server
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|e| YusraError::internal_with("inisialisasi MCP gagal", e))?;
    running
        .waiting()
        .await
        .map_err(|e| YusraError::internal_with("layanan MCP berhenti tidak normal", e))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tools_are_read_only() {
        let names: Vec<String> = YusraServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| {
                let ann = t.annotations.as_ref();
                assert_eq!(ann.and_then(|a| a.read_only_hint), Some(true), "{}", t.name);
                t.name.to_string()
            })
            .collect();
        assert_eq!(
            names
                .iter()
                .map(String::as_str)
                .collect::<std::collections::BTreeSet<_>>(),
            [
                "estimate_tokens",
                "get_project_map",
                "read_file",
                "read_symbol",
                "search_symbols"
            ]
            .into_iter()
            .collect()
        );
    }

    #[test]
    fn error_body_shape() {
        let e = YusraError::PathDenied {
            path: ".env".to_owned(),
        };
        let r = tool_error("read_file", &e);
        assert_eq!(r.is_error, Some(true));
        let body = r.structured_content.unwrap_or(Value::Null);
        assert_eq!(body["code"], "path_denied");
        assert_eq!(body["data"]["path"], ".env");

        let r = tool_error("read_file", &YusraError::internal("rahasia"));
        let body = r.structured_content.unwrap_or(Value::Null);
        assert_eq!(body["code"], "internal");
        assert!(body["data"]["trace_id"].is_string());
        assert!(!body.to_string().contains("rahasia"));
    }
}
