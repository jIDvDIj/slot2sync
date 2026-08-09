//! Operações de arquivo: listagem recursiva, download e upload.
//!
//! Uploads sempre definem `modifiedTime` = mtime local do arquivo, mantendo
//! a comparação de timestamps coerente entre máquinas. Arquivos acima de
//! `SIMPLE_UPLOAD_MAX_BYTES` usam sessão resumable.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::json;

use super::{
    ms_to_rfc3339, DriveClient, FILE_FIELDS, FOLDER_MIME_TYPE, LIST_FIELDS, OCTET_STREAM,
    SIMPLE_UPLOAD_MAX_BYTES,
};
use crate::constants::{DRIVE_APP_PROP_DEVICE, DRIVE_APP_PROP_DEVICE_ID};
use crate::error::{AppError, AppResult};
use crate::remote::{BatchUploadOp, DeviceTag, RemoteFile};

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DriveFile {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub modified_time: Option<DateTime<Utc>>,
    /// A API devolve int64 como string.
    #[serde(default)]
    pub size: Option<String>,
    /// MD5 (hex) do conteúdo, calculado pelo próprio Drive. Usado na verificação
    /// de integridade pós-download e na detecção de renomeação por conteúdo.
    #[serde(default)]
    pub md5_checksum: Option<String>,
    /// Propriedades privadas do app (ex.: `device` = quem publicou a versão).
    #[serde(default)]
    pub app_properties: HashMap<String, String>,
}

impl DriveFile {
    /// Nome amigável do dispositivo que publicou esta versão (para exibição).
    pub fn device(&self) -> Option<&str> {
        self.app_properties
            .get(DRIVE_APP_PROP_DEVICE)
            .map(String::as_str)
    }

    /// ID estável do dispositivo que publicou esta versão (para a detecção de
    /// conflito entre dispositivos). Ausente em arquivos enviados por versões
    /// antigas do app, que só gravavam o nome.
    pub fn device_id(&self) -> Option<&str> {
        self.app_properties
            .get(DRIVE_APP_PROP_DEVICE_ID)
            .map(String::as_str)
    }

    pub fn is_folder(&self) -> bool {
        self.mime_type == FOLDER_MIME_TYPE
    }

    pub fn modified_ms(&self) -> Option<i64> {
        self.modified_time.map(|t| t.timestamp_millis())
    }

    /// Converte para o `RemoteFile` genérico consumido pelo `SyncEngine`
    /// (trait `RemoteProvider`). `rel_path` não faz parte do shape do Drive —
    /// o chamador informa o que faz sentido no contexto (nome do arquivo para
    /// operações não-recursivas, caminho completo para listagem).
    pub(crate) fn to_remote(&self, rel_path: String) -> RemoteFile {
        RemoteFile {
            id: self.id.clone(),
            rel_path,
            modified_ms: self.modified_ms(),
            size_bytes: self.size.as_deref().and_then(|s| s.parse().ok()),
            hash: self.md5_checksum.clone(),
            device_name: self.device().map(str::to_string),
            device_id: self.device_id().map(str::to_string),
        }
    }
}

/// Adiciona `appProperties` (`device` = nome, `deviceId` = id) ao metadata de
/// upload, marcando a origem de cada versão no Drive. Sem nada definido, não
/// escreve a chave.
fn with_device(metadata: &mut serde_json::Value, tag: DeviceTag<'_>) {
    let mut props = serde_json::Map::new();
    if let Some(name) = tag.name {
        props.insert(DRIVE_APP_PROP_DEVICE.to_string(), name.into());
    }
    if let Some(id) = tag.id {
        props.insert(DRIVE_APP_PROP_DEVICE_ID.to_string(), id.into());
    }
    if !props.is_empty() {
        metadata["appProperties"] = serde_json::Value::Object(props);
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FileList {
    #[serde(default)]
    files: Vec<DriveFile>,
    next_page_token: Option<String>,
}

impl DriveClient {
    async fn list_children(&self, folder_id: &str) -> AppResult<Vec<DriveFile>> {
        let url = format!("{}/files", self.api_base);
        let query = format!("'{folder_id}' in parents and trashed = false");
        let mut out = Vec::new();
        let mut page_token: Option<String> = None;

        loop {
            let token_param = page_token.clone();
            let response = self
                .send_with_retry("files.list", |token| {
                    let mut request = self.http.get(&url).bearer_auth(token).query(&[
                        ("q", query.as_str()),
                        ("fields", LIST_FIELDS),
                        ("pageSize", "1000"),
                    ]);
                    if let Some(t) = token_param.as_deref() {
                        request = request.query(&[("pageToken", t)]);
                    }
                    request
                })
                .await?;

            let page: FileList = response.json().await?;
            out.extend(page.files);
            match page.next_page_token {
                Some(next) => page_token = Some(next),
                None => break,
            }
        }
        Ok(out)
    }

    /// Lista recursivamente todos os arquivos sob `folder_id`, com caminhos
    /// relativos (`sub/pasta/arquivo.ext`).
    pub async fn list_tree(&self, folder_id: &str) -> AppResult<Vec<RemoteFile>> {
        let mut out = Vec::new();
        let mut pending = vec![(folder_id.to_string(), String::new())];

        while let Some((id, prefix)) = pending.pop() {
            for child in self.list_children(&id).await? {
                let rel_path = format!("{prefix}{}", child.name);
                if child.is_folder() {
                    pending.push((child.id.clone(), format!("{rel_path}/")));
                } else {
                    out.push(child.to_remote(rel_path));
                }
            }
        }
        Ok(out)
    }

    /// Filho direto por nome (sem recursão); `mime_type` opcionalmente filtra.
    pub(crate) async fn find_child_filtered(
        &self,
        folder_id: &str,
        name: &str,
        mime_type: Option<&str>,
    ) -> AppResult<Option<DriveFile>> {
        let url = format!("{}/files", self.api_base);
        let mut query = format!("name = '{name}' and '{folder_id}' in parents and trashed = false");
        if let Some(mime) = mime_type {
            query.push_str(&format!(" and mimeType = '{mime}'"));
        }

        let response = self
            .send_with_retry("files.find", |token| {
                self.http.get(&url).bearer_auth(token).query(&[
                    ("q", query.as_str()),
                    ("fields", LIST_FIELDS),
                    ("pageSize", "1"),
                    // Determinístico: se houver duplicatas (criadas por uma
                    // versão anterior com bug de corrida), converge sempre para
                    // a mais antiga em vez de escolher uma ao acaso.
                    ("orderBy", "createdTime"),
                ])
            })
            .await?;

        let page: FileList = response.json().await?;
        Ok(page.files.into_iter().next())
    }

    pub async fn find_child(&self, folder_id: &str, name: &str) -> AppResult<Option<RemoteFile>> {
        Ok(self
            .find_child_filtered(folder_id, name, None)
            .await?
            .map(|file| {
                let rel_path = file.name.clone();
                file.to_remote(rel_path)
            }))
    }

    pub async fn download(&self, file_id: &str) -> AppResult<Vec<u8>> {
        let url = format!("{}/files/{file_id}", self.api_base);
        let response = self
            .send_with_retry("files.download", |token| {
                self.http
                    .get(&url)
                    .bearer_auth(token)
                    .query(&[("alt", "media")])
            })
            .await?;
        let content = response.bytes().await?.to_vec();
        // Compromete a janela de banda para os próximos downloads.
        self.throttle_download(content.len()).await;
        Ok(content)
    }

    /// Cria um arquivo novo em `parent_id` preservando o mtime original e
    /// marcando o dispositivo de origem em `appProperties`.
    pub async fn upload_new(
        &self,
        parent_id: &str,
        name: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        let mut metadata = json!({
            "name": name,
            "parents": [parent_id],
            "modifiedTime": ms_to_rfc3339(mtime_ms),
        });
        with_device(&mut metadata, device);
        let file = if content.len() > SIMPLE_UPLOAD_MAX_BYTES {
            let url = format!("{}/files", self.upload_base);
            self.upload_resumable(reqwest::Method::POST, &url, &metadata, content)
                .await?
        } else {
            let url = format!("{}/files", self.upload_base);
            self.upload_multipart(reqwest::Method::POST, &url, &metadata, content)
                .await?
        };
        Ok(file.to_remote(name.to_string()))
    }

    /// Atualiza o conteúdo de um arquivo existente preservando o mtime e
    /// atualizando o dispositivo de origem em `appProperties`.
    pub async fn upload_existing(
        &self,
        file_id: &str,
        content: Vec<u8>,
        mtime_ms: i64,
        device: DeviceTag<'_>,
    ) -> AppResult<RemoteFile> {
        let mut metadata = json!({ "modifiedTime": ms_to_rfc3339(mtime_ms) });
        with_device(&mut metadata, device);
        let url = format!("{}/files/{file_id}", self.upload_base);
        let file = if content.len() > SIMPLE_UPLOAD_MAX_BYTES {
            self.upload_resumable(reqwest::Method::PATCH, &url, &metadata, content)
                .await?
        } else {
            self.upload_multipart(reqwest::Method::PATCH, &url, &metadata, content)
                .await?
        };
        Ok(file.to_remote(String::new()))
    }

    /// Renomeia (e opcionalmente move de pasta) um arquivo existente via
    /// `files.update`, sem reenviar conteúdo. Usado pela detecção de
    /// renomeação por hash — evita Upload novo + zumbi do nome antigo.
    pub async fn rename_file(
        &self,
        file_id: &str,
        new_name: &str,
        add_parent: Option<&str>,
        remove_parent: Option<&str>,
    ) -> AppResult<RemoteFile> {
        let url = format!("{}/files/{file_id}", self.api_base);
        let body = json!({ "name": new_name });
        let response = self
            .send_with_retry("files.rename", |token| {
                let mut request = self
                    .http
                    .patch(&url)
                    .bearer_auth(token)
                    .query(&[("fields", FILE_FIELDS)])
                    .json(&body);
                if let Some(parent) = add_parent {
                    request = request.query(&[("addParents", parent)]);
                }
                if let Some(parent) = remove_parent {
                    request = request.query(&[("removeParents", parent)]);
                }
                request
            })
            .await?;
        let file: DriveFile = response.json().await?;
        Ok(file.to_remote(new_name.to_string()))
    }

    /// Envia até `DRIVE_BATCH_MAX_OPS` arquivos novos e pequenos em um único
    /// request `multipart/mixed`, reduzindo ~100× o número de chamadas HTTP no
    /// primeiro sync de coleções grandes. Retorna os `RemoteFile` na
    /// MESMA ordem das operações. Erro se o batch — ou qualquer sub-request —
    /// falhar; o chamador então cai no caminho per-file.
    pub async fn upload_batch(&self, ops: Vec<BatchUploadOp>) -> AppResult<Vec<RemoteFile>> {
        if ops.is_empty() {
            return Ok(Vec::new());
        }
        // Limite de banda: o batch inteiro conta como uma transferência única.
        let total_bytes: usize = ops.iter().map(|op| op.content.len()).sum();
        self.throttle_upload(total_bytes).await;
        let names: Vec<String> = ops.iter().map(|op| op.name.clone()).collect();
        let (boundary, body) = build_batch_body(&ops)?;
        let content_type = format!("multipart/mixed; boundary={boundary}");

        let response = self
            .send_with_retry("files.batchUpload", |token| {
                self.http
                    .post(&self.batch_base)
                    .bearer_auth(token)
                    .header(reqwest::header::CONTENT_TYPE, content_type.clone())
                    .body(body.clone())
            })
            .await?;

        let resp_boundary = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .and_then(parse_boundary)
            .ok_or_else(|| AppError::Other("resposta de batch sem boundary".into()))?;
        let text = response.text().await?;

        let mut items = parse_batch_response(&resp_boundary, &text)?;
        if items.len() != ops.len() {
            return Err(AppError::Other(format!(
                "batch retornou {} respostas para {} operações",
                items.len(),
                ops.len()
            )));
        }
        // A ordem das partes na resposta não é garantida; reordena pelo Content-ID.
        items.sort_by_key(|(idx, _)| *idx);
        Ok(items
            .into_iter()
            .zip(names)
            .map(|((_, file), name)| file.to_remote(name))
            .collect())
    }

    async fn upload_multipart(
        &self,
        method: reqwest::Method,
        url: &str,
        metadata: &serde_json::Value,
        content: Vec<u8>,
    ) -> AppResult<DriveFile> {
        // Limite de banda de upload: reserva a janela antes de enviar.
        self.throttle_upload(content.len()).await;
        let (boundary, body) = build_multipart_related(metadata, &content)?;
        let content_type = format!("multipart/related; boundary={boundary}");

        let response = self
            .send_with_retry("files.upload", |token| {
                self.http
                    .request(method.clone(), url)
                    .bearer_auth(token)
                    .query(&[("uploadType", "multipart"), ("fields", FILE_FIELDS)])
                    .header(reqwest::header::CONTENT_TYPE, content_type.clone())
                    .body(body.clone())
            })
            .await?;
        Ok(response.json::<DriveFile>().await?)
    }

    /// Sessão resumable: o initiate tem retry completo; o PUT do conteúdo é
    /// tentativa única — se cair, a pendência fica na fila e o próximo sync
    /// refaz a operação inteira.
    async fn upload_resumable(
        &self,
        method: reqwest::Method,
        url: &str,
        metadata: &serde_json::Value,
        content: Vec<u8>,
    ) -> AppResult<DriveFile> {
        // Limite de banda de upload: reserva a janela antes de enviar.
        self.throttle_upload(content.len()).await;
        let initiate = self
            .send_with_retry("files.upload.initiate", |token| {
                self.http
                    .request(method.clone(), url)
                    .bearer_auth(token)
                    .query(&[("uploadType", "resumable"), ("fields", FILE_FIELDS)])
                    .header("X-Upload-Content-Type", OCTET_STREAM)
                    .header("X-Upload-Content-Length", content.len().to_string())
                    .json(metadata)
            })
            .await?;

        let session_url = initiate
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::Other("upload resumable sem header Location na resposta".into())
            })?
            .to_string();

        let response = self
            .http
            .put(&session_url)
            .header(reqwest::header::CONTENT_TYPE, OCTET_STREAM)
            .body(content)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(AppError::Other(format!(
                "upload resumable falhou ({status}): {body}"
            )));
        }
        Ok(response.json::<DriveFile>().await?)
    }
}

/// Monta o corpo `multipart/related` exigido pelo upload com metadata
/// (o `multipart` do reqwest é form-data, que a API do Drive não aceita).
fn build_multipart_related(
    metadata: &serde_json::Value,
    content: &[u8],
) -> AppResult<(String, Vec<u8>)> {
    let boundary = format!("slot2sync-{:016x}", rand::random::<u64>());
    let metadata_json = serde_json::to_vec(metadata)?;

    let mut body = Vec::with_capacity(content.len() + metadata_json.len() + 256);
    body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
    body.extend_from_slice(b"Content-Type: application/json; charset=UTF-8\r\n\r\n");
    body.extend_from_slice(&metadata_json);
    body.extend_from_slice(format!("\r\n--{boundary}\r\n").as_bytes());
    body.extend_from_slice(format!("Content-Type: {OCTET_STREAM}\r\n\r\n").as_bytes());
    body.extend_from_slice(content);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

    Ok((boundary, body))
}

/// Monta o corpo `multipart/mixed` do batch: cada parte é um `application/http`
/// contendo um POST `multipart/related` (metadata + conteúdo). O `Content-ID`
/// numera cada parte para correlacionar com a resposta.
fn build_batch_body(ops: &[BatchUploadOp]) -> AppResult<(String, Vec<u8>)> {
    // `fields` precisa ir percent-encoded (vírgulas) no path literal do sub-request.
    let fields = FILE_FIELDS.replace(',', "%2C");
    let boundary = format!("slot2sync-batch-{:016x}", rand::random::<u64>());
    let mut body = Vec::new();
    for (i, op) in ops.iter().enumerate() {
        let mut metadata = json!({
            "name": op.name,
            "parents": [op.parent_id],
            "modifiedTime": ms_to_rfc3339(op.mtime_ms),
        });
        with_device(
            &mut metadata,
            DeviceTag {
                name: op.device_name.as_deref(),
                id: op.device_id.as_deref(),
            },
        );
        let (sub_boundary, related) = build_multipart_related(&metadata, &op.content)?;

        body.extend_from_slice(format!("--{boundary}\r\n").as_bytes());
        body.extend_from_slice(b"Content-Type: application/http\r\n");
        body.extend_from_slice(format!("Content-ID: <item-{i}>\r\n\r\n").as_bytes());
        body.extend_from_slice(
            format!("POST /upload/drive/v3/files?uploadType=multipart&fields={fields}\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(
            format!("Content-Type: multipart/related; boundary={sub_boundary}\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(&related);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok((boundary, body))
}

/// Extrai `boundary=...` de um Content-Type `multipart/...`.
fn parse_boundary(content_type: &str) -> Option<String> {
    content_type.split(';').find_map(|part| {
        part.trim()
            .strip_prefix("boundary=")
            .map(|b| b.trim_matches('"').to_string())
    })
}

/// Divide o corpo `multipart/mixed` da resposta e extrai `(índice, DriveFile)` de
/// cada sub-resposta. Erro se alguma sub-resposta não for 2xx.
fn parse_batch_response(boundary: &str, body: &str) -> AppResult<Vec<(usize, DriveFile)>> {
    let delim = format!("--{boundary}");
    let mut out = Vec::new();
    for part in body.split(delim.as_str()) {
        let part = part.trim_start_matches(['\r', '\n', ' ']);
        // Fecho do multipart (`--{boundary}--`) ou preâmbulo vazio.
        if part.is_empty() || part.starts_with("--") {
            continue;
        }
        // Só interessam as partes que carregam uma sub-resposta HTTP.
        if !part.contains("HTTP/") {
            continue;
        }
        if !inner_status_ok(part) {
            let status = inner_status_line(part).unwrap_or("desconhecido");
            return Err(AppError::Other(format!(
                "sub-request do batch falhou: {status}"
            )));
        }
        let idx = part
            .lines()
            .find_map(|l| l.trim().strip_prefix("Content-ID:"))
            .and_then(parse_response_index)
            .unwrap_or(out.len());
        let json = extract_json(part)
            .ok_or_else(|| AppError::Other("sub-resposta do batch sem corpo JSON".into()))?;
        let file: DriveFile = serde_json::from_str(json)?;
        out.push((idx, file));
    }
    Ok(out)
}

/// A linha de status HTTP da sub-resposta (ex.: `HTTP/1.1 200 OK`).
fn inner_status_line(part: &str) -> Option<&str> {
    part.lines().map(str::trim).find(|l| l.starts_with("HTTP/"))
}

/// `true` se a sub-resposta trouxe um status 2xx.
fn inner_status_ok(part: &str) -> bool {
    inner_status_line(part)
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|code| code.parse::<u16>().ok())
        .is_some_and(|code| (200..300).contains(&code))
}

/// Índice da parte a partir do `Content-ID` (`<response-item-3>` → `3`).
fn parse_response_index(content_id: &str) -> Option<usize> {
    content_id
        .trim()
        .trim_matches(['<', '>'])
        .rsplit('-')
        .next()?
        .parse()
        .ok()
}

/// Corpo JSON de uma parte: do primeiro `{` ao último `}` (as sub-respostas de
/// upload têm um único objeto JSON após os headers).
fn extract_json(part: &str) -> Option<&str> {
    let start = part.find('{')?;
    let end = part.rfind('}')?;
    (end >= start).then(|| &part[start..=end])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn op(name: &str, content: &[u8]) -> BatchUploadOp {
        BatchUploadOp {
            parent_id: "parent-123".into(),
            name: name.into(),
            content: content.to_vec(),
            mtime_ms: 1_700_000_000_000,
            device_name: Some("PC Gamer".into()),
            device_id: Some("dev-uuid".into()),
        }
    }

    #[test]
    fn build_batch_body_gera_partes_por_op() {
        let (boundary, body) =
            build_batch_body(&[op("a.bin", b"aaa"), op("b.bin", b"bbb")]).unwrap();
        let text = String::from_utf8_lossy(&body);

        // Duas sub-requests numeradas + fecho do multipart.
        assert_eq!(text.matches("Content-Type: application/http").count(), 2);
        assert!(text.contains("Content-ID: <item-0>"));
        assert!(text.contains("Content-ID: <item-1>"));
        assert!(text.contains("POST /upload/drive/v3/files?uploadType=multipart&fields="));
        // `fields` vai percent-encoded.
        assert!(text.contains("id%2Cname%2CmimeType"));
        assert!(text.contains(&format!("--{boundary}--")));
        // Origem do dispositivo estampada no metadata.
        assert!(text.contains("appProperties"));
        assert!(text.contains("PC Gamer"));
    }

    #[test]
    fn parse_batch_response_extrai_e_ordena_por_content_id() {
        // Resposta fora de ordem (item-1 antes de item-0): o parser reordena.
        let boundary = "batchBOUNDARY";
        let body = format!(
            "--{b}\r\n\
             Content-Type: application/http\r\n\
             Content-ID: <response-item-1>\r\n\r\n\
             HTTP/1.1 200 OK\r\n\
             Content-Type: application/json; charset=UTF-8\r\n\r\n\
             {{\"id\":\"id-B\",\"name\":\"b.bin\"}}\r\n\
             --{b}\r\n\
             Content-Type: application/http\r\n\
             Content-ID: <response-item-0>\r\n\r\n\
             HTTP/1.1 200 OK\r\n\
             Content-Type: application/json; charset=UTF-8\r\n\r\n\
             {{\"id\":\"id-A\",\"name\":\"a.bin\"}}\r\n\
             --{b}--\r\n",
            b = boundary
        );

        let mut items = parse_batch_response(boundary, &body).unwrap();
        items.sort_by_key(|(idx, _)| *idx);
        let ids: Vec<&str> = items.iter().map(|(_, f)| f.id.as_str()).collect();
        assert_eq!(ids, vec!["id-A", "id-B"]);
    }

    #[test]
    fn parse_batch_response_falha_em_sub_request_nao_2xx() {
        let boundary = "b";
        let body = format!(
            "--{b}\r\n\
             Content-Type: application/http\r\n\
             Content-ID: <response-item-0>\r\n\r\n\
             HTTP/1.1 403 Forbidden\r\n\
             Content-Type: application/json\r\n\r\n\
             {{\"error\":\"nope\"}}\r\n\
             --{b}--\r\n",
            b = boundary
        );
        assert!(parse_batch_response(boundary, &body).is_err());
    }

    #[test]
    fn parse_boundary_le_do_content_type() {
        assert_eq!(
            parse_boundary("multipart/mixed; boundary=abc123").as_deref(),
            Some("abc123")
        );
        assert_eq!(
            parse_boundary("multipart/mixed; boundary=\"quoted\"").as_deref(),
            Some("quoted")
        );
        assert_eq!(parse_boundary("application/json").as_deref(), None);
    }

    #[test]
    fn parse_response_index_le_o_numero_final() {
        assert_eq!(parse_response_index("<response-item-3>"), Some(3));
        assert_eq!(parse_response_index(" <item-0> "), Some(0));
        assert_eq!(parse_response_index("<sem-numero-x>"), None);
    }
}

/// Testes de HTTP contra um servidor fake (`wiremock`): exercitam os métodos
/// que só fazem sentido com uma requisição real (list_tree, find_child,
/// download, upload_new/existing) sem depender do Google nem de credenciais —
/// o `DriveClient` é redirecionado para `localhost` via `with_base_url`.
#[cfg(test)]
mod http_tests {
    use serde_json::json;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::SIMPLE_UPLOAD_MAX_BYTES;
    use crate::drive::test_support::client_against as test_client;
    use crate::remote::DeviceTag;

    #[tokio::test]
    async fn list_tree_percorre_subpastas_recursivamente() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .and(query_param("q", "'root-id' in parents and trashed = false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [
                    {"id": "f1", "name": "save.bin", "mimeType": "application/octet-stream"},
                    {"id": "sub1", "name": "jogo", "mimeType": "application/vnd.google-apps.folder"},
                ]
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .and(query_param("q", "'sub1' in parents and trashed = false"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [
                    {"id": "f2", "name": "state.bin", "mimeType": "application/octet-stream"},
                ]
            })))
            .mount(&server)
            .await;

        let mut rel_paths: Vec<String> = client
            .list_tree("root-id")
            .await
            .unwrap()
            .into_iter()
            .map(|f| f.rel_path)
            .collect();
        rel_paths.sort();

        assert_eq!(rel_paths, vec!["jogo/state.bin", "save.bin"]);
    }

    #[tokio::test]
    async fn find_child_retorna_o_primeiro_resultado() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .and(query_param(
                "q",
                "name = 'save.bin' and 'folder-1' in parents and trashed = false",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "files": [{"id": "abc", "name": "save.bin", "mimeType": "application/octet-stream"}]
            })))
            .mount(&server)
            .await;

        let found = client.find_child("folder-1", "save.bin").await.unwrap();
        assert_eq!(found.unwrap().id, "abc");
    }

    #[tokio::test]
    async fn find_child_sem_resultado_retorna_none() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({ "files": [] })))
            .mount(&server)
            .await;

        assert!(client
            .find_child("folder-1", "nao-existe.bin")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn rename_file_atualiza_nome_sem_reenviar_conteudo() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("PATCH"))
            .and(path("/drive/v3/files/file-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "file-1", "name": "novo.bin", "mimeType": "application/octet-stream"
            })))
            .mount(&server)
            .await;

        let renamed = client
            .rename_file("file-1", "novo.bin", None, None)
            .await
            .unwrap();
        assert_eq!(renamed.rel_path, "novo.bin");

        // O corpo enviado carrega só o nome novo — nada de conteúdo.
        let requests = server.received_requests().await.unwrap();
        let body = String::from_utf8_lossy(&requests[0].body);
        assert!(body.contains("\"name\":\"novo.bin\""));
    }

    #[tokio::test]
    async fn rename_file_com_mudanca_de_pasta_envia_parents() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("PATCH"))
            .and(path("/drive/v3/files/file-2"))
            .and(query_param("addParents", "pasta-nova"))
            .and(query_param("removeParents", "pasta-antiga"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "file-2", "name": "save.bin", "mimeType": "application/octet-stream"
            })))
            .mount(&server)
            .await;

        let renamed = client
            .rename_file(
                "file-2",
                "save.bin",
                Some("pasta-nova"),
                Some("pasta-antiga"),
            )
            .await
            .unwrap();
        assert_eq!(renamed.id, "file-2");
    }

    #[tokio::test]
    async fn download_retorna_os_bytes_do_arquivo() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("GET"))
            .and(path("/drive/v3/files/file-123"))
            .and(query_param("alt", "media"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(b"conteudo-binario".to_vec()))
            .mount(&server)
            .await;

        assert_eq!(
            client.download("file-123").await.unwrap(),
            b"conteudo-binario"
        );
    }

    #[tokio::test]
    async fn upload_new_pequeno_usa_multipart() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("POST"))
            .and(path("/upload/drive/v3/files"))
            .and(query_param("uploadType", "multipart"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "new-1",
                "name": "save.bin",
                "mimeType": "application/octet-stream",
            })))
            .mount(&server)
            .await;

        let tag = DeviceTag {
            name: Some("PC Gamer"),
            id: Some("dev-1"),
        };
        let file = client
            .upload_new(
                "parent-1",
                "save.bin",
                b"dados".to_vec(),
                1_700_000_000_000,
                tag,
            )
            .await
            .unwrap();

        assert_eq!(file.id, "new-1");
    }

    #[tokio::test]
    async fn upload_existing_pequeno_usa_multipart_patch() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("PATCH"))
            .and(path("/upload/drive/v3/files/file-9"))
            .and(query_param("uploadType", "multipart"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "file-9",
                "name": "save.bin",
                "mimeType": "application/octet-stream",
            })))
            .mount(&server)
            .await;

        let file = client
            .upload_existing(
                "file-9",
                b"novo".to_vec(),
                1_700_000_000_000,
                DeviceTag::default(),
            )
            .await
            .unwrap();

        assert_eq!(file.id, "file-9");
    }

    /// Arquivo acima do limite de multipart usa sessão resumable: POST inicia
    /// (devolve a URL da sessão no header `Location`) e o conteúdo vai num PUT
    /// separado para essa URL.
    #[tokio::test]
    async fn upload_new_grande_usa_sessao_resumable() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;
        let session_url = format!("{}/resumable-session/abc", server.uri());

        Mock::given(method("POST"))
            .and(path("/upload/drive/v3/files"))
            .and(query_param("uploadType", "resumable"))
            .respond_with(
                ResponseTemplate::new(200).insert_header("Location", session_url.as_str()),
            )
            .mount(&server)
            .await;

        Mock::given(method("PUT"))
            .and(path("/resumable-session/abc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id": "big-1",
                "name": "save.bin",
                "mimeType": "application/octet-stream",
            })))
            .mount(&server)
            .await;

        let big_content = vec![0u8; SIMPLE_UPLOAD_MAX_BYTES + 1];
        let file = client
            .upload_new(
                "parent-1",
                "save.bin",
                big_content,
                1_700_000_000_000,
                DeviceTag::default(),
            )
            .await
            .unwrap();

        assert_eq!(file.id, "big-1");
    }
}
