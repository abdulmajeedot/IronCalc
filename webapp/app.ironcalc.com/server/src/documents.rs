//! Workbooks kept on the server, for one owner who proves it with a token.
//!
//! The server links the engine, so it checks what it is given is a workbook
//! and can read calculated values out of one.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{env, fs, io, path::PathBuf};

use ironcalc::base::cell::CellValue;
use ironcalc::base::expressions::utils::parse_reference_a1;
use ironcalc::base::preserved::is_external_cache_sheet;
use ironcalc::base::types::CellType;
use ironcalc::base::Model as IModel;
use rand::RngCore;
use rocket::data::{Data, ToByteUnit};
use rocket::fairing::{self, AdHoc};
use rocket::http::{Header, Status};
use rocket::request::{FromRequest, Outcome, Request};
use rocket::serde::json::{serde_json::json, Json, Value};
use rocket::serde::{Deserialize, Serialize};
use rocket::{Build, Rocket};
use rocket_db_pools::sqlx::Acquire;
use rocket_db_pools::{sqlx, Connection, Database};

use crate::database::IronCalcDB;
use crate::id;

const MAX_DOCUMENT_MB: u64 = 20;
const MAX_VALUE_CELLS: i64 = 100_000;
const MAX_NAME_LENGTH: usize = 200;
const TOKEN_FILE: &str = ".api-token";

/// The one token that opens the documents.
pub struct ApiToken(pub String);

/// The token from `IRONCALC_API_TOKEN`, else from a file, which is created
/// with a new random token the first time.
pub fn load_token() -> io::Result<String> {
    if let Ok(token) = env::var("IRONCALC_API_TOKEN") {
        if !token.trim().is_empty() {
            return Ok(token.trim().to_string());
        }
    }
    let path = env::var("IRONCALC_API_TOKEN_FILE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from(TOKEN_FILE));
    if let Ok(token) = fs::read_to_string(&path) {
        if !token.trim().is_empty() {
            return Ok(token.trim().to_string());
        }
    }
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let token: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    fs::write(&path, &token)?;
    restrict_to_owner(&path)?;
    println!(
        "Created a token for the documents API in {}",
        path.display()
    );
    Ok(token)
}

#[cfg(unix)]
fn restrict_to_owner(path: &PathBuf) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn restrict_to_owner(_path: &PathBuf) -> io::Result<()> {
    Ok(())
}

fn same_token(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// A request that carries the token.
pub struct Authorized;

#[rocket::async_trait]
impl<'r> FromRequest<'r> for Authorized {
    type Error = ();

    async fn from_request(request: &'r Request<'_>) -> Outcome<Self, ()> {
        let Some(expected) = request.rocket().state::<ApiToken>() else {
            return Outcome::Error((Status::Unauthorized, ()));
        };
        let given = request
            .headers()
            .get_one("Authorization")
            .and_then(|value| value.strip_prefix("Bearer "));
        match given {
            Some(token) if same_token(token.as_bytes(), expected.0.as_bytes()) => {
                Outcome::Success(Authorized)
            }
            _ => Outcome::Error((Status::Unauthorized, ())),
        }
    }
}

#[derive(Serialize, Deserialize, Debug, PartialEq, Clone)]
#[serde(crate = "rocket::serde")]
pub struct DocumentInfo {
    pub id: String,
    pub name: String,
    pub version: i64,
    pub updated_at: i64,
    /// The documents this one reads values from.
    #[serde(default)]
    pub sources: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(crate = "rocket::serde")]
pub struct Saved {
    pub version: i64,
}

#[derive(Serialize, Deserialize, Debug, PartialEq)]
#[serde(crate = "rocket::serde")]
pub struct Conflict {
    pub current_version: i64,
}

#[derive(Deserialize)]
#[serde(crate = "rocket::serde")]
pub struct Rename {
    name: String,
}

#[derive(rocket::Responder)]
pub struct DocumentBytes {
    inner: Vec<u8>,
    version: Header<'static>,
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_millis() as i64)
}

fn clean_name(name: &str) -> Result<String, Status> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_LENGTH {
        return Err(Status::BadRequest);
    }
    Ok(name.to_string())
}

fn internal(error: sqlx::Error) -> Status {
    eprintln!("Documents database error: {error}");
    Status::InternalServerError
}

/// The bytes of a request, if they are a workbook, and the documents it reads.
async fn workbook_bytes(data: Data<'_>) -> Result<(Vec<u8>, Vec<String>), Status> {
    let bytes = data
        .open(MAX_DOCUMENT_MB.mebibytes())
        .into_bytes()
        .await
        .map_err(|_| Status::BadRequest)?;
    if !bytes.is_complete() {
        return Err(Status::PayloadTooLarge);
    }
    let bytes = bytes.into_inner();
    let model = IModel::from_bytes(&bytes, "en").map_err(|_| Status::UnprocessableEntity)?;
    let sources = model
        .workbook
        .external_links
        .iter()
        .filter_map(|link| link.document_id.clone())
        .collect();
    drop(model);
    Ok((bytes, sources))
}

async fn replace_sources(
    connection: &mut sqlx::SqliteConnection,
    id: &str,
    sources: &[String],
) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM document_sources WHERE document_id = ?")
        .bind(id)
        .execute(&mut *connection)
        .await?;
    for source in sources {
        sqlx::query(
            "INSERT OR IGNORE INTO document_sources (document_id, source_id) VALUES (?, ?)",
        )
        .bind(id)
        .bind(source)
        .execute(&mut *connection)
        .await?;
    }
    Ok(())
}

#[get("/api/documents")]
async fn list_documents(
    _authorized: Authorized,
    mut db: Connection<IronCalcDB>,
) -> Result<Json<Vec<DocumentInfo>>, Status> {
    let rows: Vec<(String, String, i64, i64)> = sqlx::query_as(
        "SELECT id, name, version, updated_at FROM documents ORDER BY updated_at DESC",
    )
    .fetch_all(&mut **db)
    .await
    .map_err(internal)?;
    let edges: Vec<(String, String)> =
        sqlx::query_as("SELECT document_id, source_id FROM document_sources")
            .fetch_all(&mut **db)
            .await
            .map_err(internal)?;
    let mut sources: HashMap<String, Vec<String>> = HashMap::new();
    for (document, source) in edges {
        sources.entry(document).or_default().push(source);
    }
    Ok(Json(
        rows.into_iter()
            .map(|(id, name, version, updated_at)| DocumentInfo {
                sources: sources.remove(&id).unwrap_or_default(),
                id,
                name,
                version,
                updated_at,
            })
            .collect(),
    ))
}

#[post("/api/documents?<name>", data = "<data>")]
async fn create_document(
    _authorized: Authorized,
    mut db: Connection<IronCalcDB>,
    name: &str,
    data: Data<'_>,
) -> Result<(Status, Json<DocumentInfo>), Status> {
    let name = clean_name(name)?;
    let (bytes, sources) = workbook_bytes(data).await?;
    let info = DocumentInfo {
        id: id::new_id(),
        name,
        version: 1,
        updated_at: now(),
        sources: sources.clone(),
    };
    let mut transaction = (&mut **db).begin().await.map_err(internal)?;
    sqlx::query(
        "INSERT INTO documents (id, name, version, bytes, created_at, updated_at) VALUES (?, ?, 1, ?, ?, ?)",
    )
    .bind(&info.id)
    .bind(&info.name)
    .bind(&bytes)
    .bind(info.updated_at)
    .bind(info.updated_at)
    .execute(&mut *transaction)
    .await
    .map_err(internal)?;
    replace_sources(&mut transaction, &info.id, &sources)
        .await
        .map_err(internal)?;
    transaction.commit().await.map_err(internal)?;
    Ok((Status::Created, Json(info)))
}

#[get("/api/documents/<id>")]
async fn get_document(
    _authorized: Authorized,
    mut db: Connection<IronCalcDB>,
    id: &str,
) -> Result<DocumentBytes, Status> {
    let row: Option<(Vec<u8>, i64)> =
        sqlx::query_as("SELECT bytes, version FROM documents WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut **db)
            .await
            .map_err(internal)?;
    let (inner, version) = row.ok_or(Status::NotFound)?;
    Ok(DocumentBytes {
        inner,
        version: Header::new("X-Document-Version", version.to_string()),
    })
}

/// Replaces a document if `version` is still its version, so a save can't
/// wipe out one it hasn't seen.
#[put("/api/documents/<id>?<version>&<name>", data = "<data>")]
async fn save_document(
    _authorized: Authorized,
    mut db: Connection<IronCalcDB>,
    id: &str,
    version: i64,
    name: Option<&str>,
    data: Data<'_>,
) -> Result<Json<Saved>, (Status, Option<Json<Conflict>>)> {
    let name = name
        .map(clean_name)
        .transpose()
        .map_err(|status| (status, None))?;
    let (bytes, sources) = workbook_bytes(data)
        .await
        .map_err(|status| (status, None))?;
    let mut transaction = (&mut **db).begin().await.map_err(|e| (internal(e), None))?;
    let result = sqlx::query(
        "UPDATE documents SET bytes = ?, version = version + 1, updated_at = ?, name = COALESCE(?, name) WHERE id = ? AND version = ?",
    )
    .bind(&bytes)
    .bind(now())
    .bind(name)
    .bind(id)
    .bind(version)
    .execute(&mut *transaction)
    .await
    .map_err(|e| (internal(e), None))?;
    if result.rows_affected() == 1 {
        replace_sources(&mut transaction, id, &sources)
            .await
            .map_err(|e| (internal(e), None))?;
        transaction
            .commit()
            .await
            .map_err(|e| (internal(e), None))?;
        return Ok(Json(Saved {
            version: version + 1,
        }));
    }
    drop(transaction);
    let current: Option<(i64,)> = sqlx::query_as("SELECT version FROM documents WHERE id = ?")
        .bind(id)
        .fetch_optional(&mut **db)
        .await
        .map_err(|e| (internal(e), None))?;
    match current {
        Some((current_version,)) => {
            Err((Status::Conflict, Some(Json(Conflict { current_version }))))
        }
        None => Err((Status::NotFound, None)),
    }
}

#[patch("/api/documents/<id>", data = "<rename>")]
async fn rename_document(
    _authorized: Authorized,
    mut db: Connection<IronCalcDB>,
    id: &str,
    rename: Json<Rename>,
) -> Result<Status, Status> {
    let name = clean_name(&rename.name)?;
    let result = sqlx::query("UPDATE documents SET name = ? WHERE id = ?")
        .bind(name)
        .bind(id)
        .execute(&mut **db)
        .await
        .map_err(internal)?;
    Ok(if result.rows_affected() == 1 {
        Status::NoContent
    } else {
        Status::NotFound
    })
}

#[delete("/api/documents/<id>")]
async fn delete_document(
    _authorized: Authorized,
    mut db: Connection<IronCalcDB>,
    id: &str,
) -> Result<Status, Status> {
    sqlx::query("DELETE FROM document_sources WHERE document_id = ?")
        .bind(id)
        .execute(&mut **db)
        .await
        .map_err(internal)?;
    let result = sqlx::query("DELETE FROM documents WHERE id = ?")
        .bind(id)
        .execute(&mut **db)
        .await
        .map_err(internal)?;
    Ok(if result.rows_affected() == 1 {
        Status::NoContent
    } else {
        Status::NotFound
    })
}

/// `A1:B2` as first row, first column, last row, last column.
fn parse_area(range: &str) -> Option<(i32, i32, i32, i32)> {
    let upper = range.to_uppercase().replace('$', "");
    let (first, last) = upper.split_once(':').unwrap_or((&upper, &upper));
    let a = parse_reference_a1(first)?;
    let b = parse_reference_a1(last)?;
    Some((
        a.row.min(b.row),
        a.column.min(b.column),
        a.row.max(b.row),
        a.column.max(b.column),
    ))
}

/// The calculated values of a range, row by row.
#[get("/api/documents/<id>/values?<sheet>&<range>")]
async fn document_values(
    _authorized: Authorized,
    mut db: Connection<IronCalcDB>,
    id: &str,
    sheet: &str,
    range: &str,
) -> Result<Json<Value>, Status> {
    let row: Option<(Vec<u8>, i64)> =
        sqlx::query_as("SELECT bytes, version FROM documents WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut **db)
            .await
            .map_err(internal)?;
    let (bytes, version) = row.ok_or(Status::NotFound)?;
    let (first_row, first_column, last_row, last_column) =
        parse_area(range).ok_or(Status::BadRequest)?;
    let cells = i64::from(last_row - first_row + 1) * i64::from(last_column - first_column + 1);
    if cells > MAX_VALUE_CELLS {
        return Err(Status::PayloadTooLarge);
    }
    let model = IModel::from_bytes(&bytes, "en").map_err(|_| Status::InternalServerError)?;
    let index = model
        .workbook
        .worksheets
        .iter()
        .position(|worksheet| worksheet.name.eq_ignore_ascii_case(sheet))
        .ok_or(Status::NotFound)? as u32;
    let mut values = Vec::new();
    for row in first_row..=last_row {
        let mut cells = Vec::new();
        for column in first_column..=last_column {
            cells.push(
                match model
                    .get_cell_value_by_index(index, row, column)
                    .map_err(|_| Status::InternalServerError)?
                {
                    CellValue::None => Value::Null,
                    CellValue::Number(number) => json!(number),
                    CellValue::String(text) => json!(text),
                    CellValue::Boolean(value) => json!(value),
                },
            );
        }
        values.push(cells);
    }
    Ok(Json(json!({ "version": version, "values": values })))
}

/// Every filled cell of a document, for the workbooks that link to it.
#[get("/api/documents/<id>/snapshot")]
async fn document_snapshot(
    _authorized: Authorized,
    mut db: Connection<IronCalcDB>,
    id: &str,
) -> Result<Json<Value>, Status> {
    let row: Option<(Vec<u8>, i64)> =
        sqlx::query_as("SELECT bytes, version FROM documents WHERE id = ?")
            .bind(id)
            .fetch_optional(&mut **db)
            .await
            .map_err(internal)?;
    let (bytes, version) = row.ok_or(Status::NotFound)?;
    let model = IModel::from_bytes(&bytes, "en").map_err(|_| Status::InternalServerError)?;
    let mut sheets = Vec::new();
    for (index, worksheet) in model.workbook.worksheets.iter().enumerate() {
        if is_external_cache_sheet(worksheet) {
            continue;
        }
        let mut cells = Vec::new();
        for (row, column, cell) in worksheet.sheet_data.cells() {
            let value = model
                .get_cell_value_by_index(index as u32, row, column)
                .map_err(|_| Status::InternalServerError)?;
            let is_error = cell.get_type() == CellType::ErrorValue;
            match value {
                CellValue::None => {}
                CellValue::String(text) if is_error => {
                    cells.push(json!([row, column, { "error": text }]));
                }
                CellValue::Number(number) => cells.push(json!([row, column, number])),
                CellValue::String(text) => cells.push(json!([row, column, text])),
                CellValue::Boolean(value) => cells.push(json!([row, column, value])),
            }
        }
        sheets.push(json!({ "name": worksheet.name, "cells": cells }));
    }
    Ok(Json(json!({ "version": version, "sheets": sheets })))
}

async fn create_table(rocket: Rocket<Build>) -> fairing::Result {
    let Some(pool) = IronCalcDB::fetch(&rocket) else {
        return Err(rocket);
    };
    let created = sqlx::query(
        "CREATE TABLE IF NOT EXISTS documents (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            version INTEGER NOT NULL,
            bytes BLOB NOT NULL,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        )",
    )
    .execute(&**pool)
    .await;
    let created = match created {
        Ok(_) => {
            sqlx::query(
                "CREATE TABLE IF NOT EXISTS document_sources (
                    document_id TEXT NOT NULL,
                    source_id TEXT NOT NULL,
                    PRIMARY KEY (document_id, source_id)
                )",
            )
            .execute(&**pool)
            .await
        }
        error => error,
    };
    match created {
        Ok(_) => Ok(rocket),
        Err(error) => {
            eprintln!("Could not create the documents table: {error}");
            Err(rocket)
        }
    }
}

/// Adds the documents API to a server with the database attached.
pub fn stage(token: String) -> AdHoc {
    AdHoc::on_ignite("Documents", |rocket| async {
        rocket
            .manage(ApiToken(token))
            .attach(AdHoc::try_on_ignite("Documents table", create_table))
            .mount(
                "/",
                routes![
                    list_documents,
                    create_document,
                    get_document,
                    save_document,
                    rename_document,
                    delete_document,
                    document_values,
                    document_snapshot,
                ],
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rocket::http::ContentType;
    use rocket::local::asynchronous::{Client, LocalResponse};

    const TOKEN: &str = "test-token-for-the-documents-api";

    fn workbook() -> Vec<u8> {
        let mut model = IModel::new_empty("book", "en", "UTC", "en").unwrap();
        model.set_user_input(0, 1, 1, "5".to_string()).unwrap();
        model.set_user_input(0, 2, 1, "=A1*2".to_string()).unwrap();
        model.set_user_input(0, 1, 2, "text".to_string()).unwrap();
        model.evaluate();
        model.to_bytes()
    }

    async fn client(label: &str) -> Client {
        let path = env::temp_dir().join(format!(
            "ironcalc-doc-{label}-{}.sqlite",
            id::new_id().replace('!', "_")
        ));
        fs::write(&path, b"").unwrap();
        let figment = rocket::Config::figment().merge((
            "databases.ironcalc.url",
            path.to_string_lossy().into_owned(),
        ));
        let rocket = rocket::custom(figment)
            .attach(IronCalcDB::init())
            .attach(stage(TOKEN.to_string()));
        Client::tracked(rocket).await.unwrap()
    }

    fn bearer() -> Header<'static> {
        Header::new("Authorization", format!("Bearer {TOKEN}"))
    }

    async fn create(client: &Client, name: &str) -> DocumentInfo {
        let response = client
            .post(format!("/api/documents?name={name}"))
            .header(bearer())
            .header(ContentType::Binary)
            .body(workbook())
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Created);
        response.into_json().await.unwrap()
    }

    async fn save<'c>(client: &'c Client, id: &str, version: i64) -> LocalResponse<'c> {
        client
            .put(format!("/api/documents/{id}?version={version}"))
            .header(bearer())
            .body(workbook())
            .dispatch()
            .await
    }

    #[rocket::async_test]
    async fn every_route_needs_the_token() {
        let client = client("auth").await;
        let doc = create(&client, "Book").await;
        let wrong = Header::new("Authorization", "Bearer nope");
        for path in [
            "/api/documents".to_string(),
            format!("/api/documents/{}", doc.id),
            format!("/api/documents/{}/values?sheet=Sheet1&range=A1", doc.id),
        ] {
            let plain = client.get(path.clone()).dispatch().await;
            assert_eq!(plain.status(), Status::Unauthorized, "{path}");
            let bad = client
                .get(path.clone())
                .header(wrong.clone())
                .dispatch()
                .await;
            assert_eq!(bad.status(), Status::Unauthorized, "{path}");
        }
        let delete = client
            .delete(format!("/api/documents/{}", doc.id))
            .dispatch()
            .await;
        assert_eq!(delete.status(), Status::Unauthorized);
        let put = client
            .put(format!("/api/documents/{}?version=1", doc.id))
            .body(workbook())
            .dispatch()
            .await;
        assert_eq!(put.status(), Status::Unauthorized);
    }

    #[rocket::async_test]
    async fn a_document_comes_back_with_its_version() {
        let client = client("roundtrip").await;
        let doc = create(&client, "Book").await;
        assert_eq!(doc.version, 1);
        let response = client
            .get(format!("/api/documents/{}", doc.id))
            .header(bearer())
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        assert_eq!(response.headers().get_one("X-Document-Version"), Some("1"));
        assert_eq!(response.into_bytes().await.unwrap(), workbook());
    }

    #[rocket::async_test]
    async fn a_stale_save_is_refused() {
        let client = client("conflict").await;
        let doc = create(&client, "Book").await;
        let first = save(&client, &doc.id, 1).await;
        assert_eq!(first.status(), Status::Ok);
        assert_eq!(first.into_json::<Saved>().await.unwrap().version, 2);

        let stale = save(&client, &doc.id, 1).await;
        assert_eq!(stale.status(), Status::Conflict);
        assert_eq!(
            stale.into_json::<Conflict>().await.unwrap().current_version,
            2
        );
        assert_eq!(save(&client, &doc.id, 2).await.status(), Status::Ok);
    }

    #[rocket::async_test]
    async fn only_workbooks_are_stored() {
        let client = client("invalid").await;
        let response = client
            .post("/api/documents?name=Bad")
            .header(bearer())
            .body(b"not a workbook".to_vec())
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::UnprocessableEntity);
        let list = client
            .get("/api/documents")
            .header(bearer())
            .dispatch()
            .await;
        assert!(list
            .into_json::<Vec<DocumentInfo>>()
            .await
            .unwrap()
            .is_empty());
    }

    #[rocket::async_test]
    async fn the_list_shows_names_and_a_rename_sticks() {
        let client = client("list").await;
        let doc = create(&client, "First").await;
        let response = client
            .patch(format!("/api/documents/{}", doc.id))
            .header(bearer())
            .header(ContentType::JSON)
            .body(r#"{"name":"Renamed"}"#)
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::NoContent);
        let list = client
            .get("/api/documents")
            .header(bearer())
            .dispatch()
            .await;
        let list = list.into_json::<Vec<DocumentInfo>>().await.unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Renamed");
    }

    #[rocket::async_test]
    async fn a_deleted_document_is_gone() {
        let client = client("delete").await;
        let doc = create(&client, "Book").await;
        let path = format!("/api/documents/{}", doc.id);
        let deleted = client
            .delete(path.clone())
            .header(bearer())
            .dispatch()
            .await;
        assert_eq!(deleted.status(), Status::NoContent);
        let gone = client.get(path).header(bearer()).dispatch().await;
        assert_eq!(gone.status(), Status::NotFound);
        assert_eq!(save(&client, &doc.id, 1).await.status(), Status::NotFound);
    }

    #[rocket::async_test]
    async fn the_list_says_which_documents_a_document_reads() {
        let client = client("sources").await;
        let source = create(&client, "Source").await;
        let mut reader = IModel::new_empty("reader", "en", "UTC", "en").unwrap();
        reader
            .set_linked_workbook("Source", &source.id, 1, Vec::new())
            .unwrap();
        let response = client
            .post("/api/documents?name=Reader")
            .header(bearer())
            .body(reader.to_bytes())
            .dispatch()
            .await;
        let created: DocumentInfo = response.into_json().await.unwrap();
        assert_eq!(created.sources, vec![source.id.clone()]);

        let list = client
            .get("/api/documents")
            .header(bearer())
            .dispatch()
            .await;
        let list = list.into_json::<Vec<DocumentInfo>>().await.unwrap();
        let entry = list.iter().find(|d| d.id == created.id).unwrap();
        assert_eq!(entry.sources, vec![source.id.clone()]);
        assert!(list
            .iter()
            .find(|d| d.id == source.id)
            .unwrap()
            .sources
            .is_empty());

        assert_eq!(save(&client, &created.id, 1).await.status(), Status::Ok);
        let list = client
            .get("/api/documents")
            .header(bearer())
            .dispatch()
            .await;
        let list = list.into_json::<Vec<DocumentInfo>>().await.unwrap();
        assert!(list
            .iter()
            .find(|d| d.id == created.id)
            .unwrap()
            .sources
            .is_empty());
    }

    #[rocket::async_test]
    async fn a_snapshot_has_every_filled_cell() {
        let client = client("snapshot").await;
        let mut model = IModel::new_empty("book", "en", "UTC", "en").unwrap();
        model.set_user_input(0, 1, 1, "=1/0".to_string()).unwrap();
        model.set_user_input(0, 2, 2, "text".to_string()).unwrap();
        model.set_user_input(0, 3, 1, "=2+3".to_string()).unwrap();
        model.evaluate();
        let response = client
            .post("/api/documents?name=Source")
            .header(bearer())
            .body(model.to_bytes())
            .dispatch()
            .await;
        let doc: DocumentInfo = response.into_json().await.unwrap();
        let response = client
            .get(format!("/api/documents/{}/snapshot", doc.id))
            .header(bearer())
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        let body: Value = response.into_json().await.unwrap();
        assert_eq!(body["version"], 1);
        assert_eq!(body["sheets"][0]["name"], "Sheet1");
        let cells = body["sheets"][0]["cells"].as_array().unwrap();
        assert_eq!(cells.len(), 3);
        assert!(cells.contains(&json!([1, 1, { "error": "#DIV/0!" }])));
        assert!(cells.contains(&json!([2, 2, "text"])));
        assert!(cells.contains(&json!([3, 1, 5.0])));

        let plain = client
            .get(format!("/api/documents/{}/snapshot", doc.id))
            .dispatch()
            .await;
        assert_eq!(plain.status(), Status::Unauthorized);
    }

    #[rocket::async_test]
    async fn calculated_values_can_be_read() {
        let client = client("values").await;
        let doc = create(&client, "Book").await;
        let response = client
            .get(format!(
                "/api/documents/{}/values?sheet=sheet1&range=$A$1:B2",
                doc.id
            ))
            .header(bearer())
            .dispatch()
            .await;
        assert_eq!(response.status(), Status::Ok);
        let body: Value = response.into_json().await.unwrap();
        assert_eq!(body["version"], 1);
        assert_eq!(body["values"], json!([[5.0, "text"], [10.0, null]]));

        for (sheet, range, status) in [
            ("Nope", "A1", Status::NotFound),
            ("Sheet1", "not a range", Status::BadRequest),
            ("Sheet1", "A1:Z1048576", Status::PayloadTooLarge),
        ] {
            let response = client
                .get(format!(
                    "/api/documents/{}/values?sheet={sheet}&range={}",
                    doc.id,
                    range.replace(' ', "%20")
                ))
                .header(bearer())
                .dispatch()
                .await;
            assert_eq!(response.status(), status, "{sheet} {range}");
        }
    }
}
