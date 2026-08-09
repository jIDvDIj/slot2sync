use std::path::Path;

/// Caminho do `.env` relativo a `src-tauri/` (cwd dos build scripts).
const DOTENV_PATH: &str = "../.env";

/// Prefixo das variáveis que podem ser embutidas no binário.
const ENV_PREFIX: &str = "SLOT2SYNC_";

/// Variáveis lidas por `option_env!` no código (`auth/oauth.rs`). Precisam ser
/// declaradas como dependências de build **sempre** — inclusive no CI, onde não
/// existe `.env` e o early-return de `load_dotenv` não emitiria nada. Sem isso,
/// o cache do cargo (`rust-cache`) pode servir um binário com credenciais antigas
/// quando uma delas é rotacionada, pois nada invalida a recompilação do crate.
const EMBEDDED_KEYS: &[&str] = &[
    "SLOT2SYNC_GOOGLE_CLIENT_ID",
    "SLOT2SYNC_GOOGLE_CLIENT_SECRET",
    "SLOT2SYNC_TOKEN_PROXY_URL",
    "SLOT2SYNC_PROXY_SECRET",
    "SLOT2SYNC_DROPBOX_CLIENT_ID",
    "SLOT2SYNC_DROPBOX_TOKEN_PROXY_URL",
    "SLOT2SYNC_ONEDRIVE_CLIENT_ID",
    "SLOT2SYNC_ONEDRIVE_TOKEN_PROXY_URL",
];

fn main() {
    for key in EMBEDDED_KEYS {
        println!("cargo:rerun-if-env-changed={key}");
    }
    load_dotenv();
    tauri_build::build()
}

/// Lê o `.env` da raiz do repositório e reexporta as variáveis `SLOT2SYNC_*`
/// via `cargo:rustc-env`, tornando-as visíveis ao `option_env!` do código.
/// Variáveis já definidas no ambiente do shell têm precedência sobre o arquivo.
fn load_dotenv() {
    println!("cargo:rerun-if-changed={DOTENV_PATH}");

    let Ok(content) = std::fs::read_to_string(Path::new(DOTENV_PATH)) else {
        return;
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if !key.starts_with(ENV_PREFIX) {
            continue;
        }
        println!("cargo:rerun-if-env-changed={key}");
        if std::env::var(key).is_ok() {
            continue;
        }
        let value = value.trim().trim_matches('"').trim_matches('\'');
        if value.is_empty() {
            continue;
        }
        println!("cargo:rustc-env={key}={value}");
    }
}
