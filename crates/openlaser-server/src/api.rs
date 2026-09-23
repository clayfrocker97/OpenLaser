// SPDX-License-Identifier: GPL-3.0-or-later

//! The HTTP API.
//!
//! Every command is a `POST` with a JSON body, or the raw bytes of a file,
//! and answers `{ok: true}` or `{error: "..."}`. Long operations answer as
//! soon as they are started; their progress is in the state. The state is
//! served whole at `/api/state` and pushed as server-sent events from
//! `/api/events`, where the library, draft, bindings and files sections
//! are sent only when they change.

use crate::connect::{self, RouteChange};
use crate::coordinator::{ItemChange, NewRecipe, RecipeChange, RecipeImport, Shared};
use crate::machine;
use crate::ui::{self, UiDir};
use crate::{Error, document::Document};
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};

pub(crate) mod browser;
mod gas;
mod machine_actions;
use openlaser_core::features::Features;
use openlaser_core::geometry::Transform;
use openlaser_library::Id;
use serde::Deserialize;
use serde_json::{Value, json};

/// The router with embedded assets, or `ui_dir` for development builds.
pub fn router(shared: Shared, ui_dir: std::path::PathBuf) -> Router {
    Router::new()
        .route("/api/state", get(state))
        .route(
            "/api/fonts",
            get(fonts).post(import_font).layer(DefaultBodyLimit::max(openlaser_svg::MAX_BYTES)),
        )
        .route("/api/text/preview", post(preview_text))
        .route("/api/parts/text", post(import_text))
        .route("/api/library/history", get(edit_history))
        .route("/api/library/history/{action}", post(undo_saved))
        .route("/api/jobs/{id}/duplicate", post(duplicate_job))
        .route("/api/events", get(events))
        .route("/api/alarms/history", get(alarm_history))
        .route("/api/preflight", get(preflight_review))
        .route(
            "/api/preflight/preferences",
            get(preflight_preferences).post(save_preflight_preferences),
        )
        .route("/api/preflight/action", post(preflight_action))
        .route("/api/touch", post(save_touch))
        .route("/api/postflight", get(postflight_review))
        .route("/api/postflight/action", post(postflight_action))
        .route("/api/postflight/dismiss", post(dismiss_postflight))
        .route("/api/sheets", get(sheet_history))
        .route("/api/sheets/{id}", get(sheet_view).post(save_remnant))
        .route("/api/jobs/{id}/cut-sheet", post(report_cut_sheet))
        .route("/api/parts", post(import_part))
        .route("/api/parts/review", post(review_part))
        .route("/api/parts/{id}", post(update_part).delete(remove_part))
        .route("/api/parts/{id}/duplicate", post(duplicate_part))
        .route("/api/parts/{id}/simplify", post(simplify_part))
        .route("/api/folders", post(add_folder))
        .route("/api/folders/{id}", post(update_folder).delete(remove_folder))
        .route("/api/recipes", post(add_recipe))
        .route("/api/recipes/import", post(import_recipe))
        .route("/api/recipes/import/preview", post(preview_recipe))
        .route("/api/recipes/{id}", post(update_recipe).delete(remove_recipe))
        .route("/api/recipes/{id}/photo", post(set_photo))
        .route("/api/recipes/{id}/duplicate", post(duplicate_recipe))
        .route("/api/photos/{sha256}", get(photo))
        .route("/api/machine/files", post(import_machine_file))
        .route("/api/machine/settings", get(machine_settings).post(save_machine_settings))
        .route("/api/machine/correction", get(correction_view).post(correction_save))
        .route("/api/machine/correction/coupon", post(correction_coupon))
        .route("/api/machine/settings/read", post(read_machine_settings))
        .route("/api/machine/settings/write", post(write_machine_settings))
        .route("/api/machine/soft", get(download_soft).post(import_soft))
        .route("/api/machine/route", post(set_route))
        .route("/api/jobs/{id}", post(update_job).delete(remove_job))
        .route("/api/draft/part/{id}", post(open_part))
        .route("/api/draft/parts", post(open_parts))
        .route("/api/draft/parts/add", post(add_parts))
        .route("/api/drafts", get(pending_drafts))
        .route("/api/drafts/{key}", post(open_retained).delete(discard_draft))
        .route("/api/draft/merge", get(merge_review).post(resolve_merge))
        .route("/api/draft/job/{id}", post(open_job))
        .route("/api/draft/recipe/{id}", post(set_recipe))
        .route("/api/draft/features", post(set_features))
        .route("/api/draft/preflight", post(set_preflight))
        .route("/api/draft/copy/{id}", post(copy_features))
        .route("/api/draft/transform", post(transform))
        .route("/api/draft/add", post(add))
        .route("/api/draft/stock", post(set_stock))
        .route("/api/draft/sheet/{index}", post(select_sheet))
        .route("/api/draft/nest/{id}/sheet/{index}", get(nest_sheet))
        .route("/api/draft/nest", post(nest_start))
        .route("/api/draft/nest/{id}", get(nest_status).delete(nest_cancel))
        .route("/api/draft/nest/{id}/apply", post(nest_apply))
        .route("/api/draft/remove", post(remove))
        .route("/api/draft/group", post(group))
        .route("/api/draft/ungroup", post(ungroup))
        .route("/api/draft/undo", post(undo))
        .route("/api/draft/redo", post(redo))
        .route("/api/recovery", post(recovery_change))
        .route("/api/recovery/program", get(recovery_program))
        .route("/api/recovery/prepare", post(recovery_prepare))
        .route("/api/recovery/move", post(recovery_move))
        .route("/api/draft/pick", post(pick))
        .route("/api/draft/preview", post(preview_features))
        .route("/api/draft/origin", post(set_origin))
        .route("/api/draft/placement", post(set_placement))
        .route("/api/draft/placement/prepare", post(prepare_placement))
        .route("/api/draft/anchor", post(set_anchor))
        .route("/api/draft/compile", post(compile))
        .route("/api/draft/save", post(save_job))
        .merge(machine_actions::routes())
        .merge(gas::routes())
        .layer(DefaultBodyLimit::max(UPLOAD_LIMIT))
        .with_state(shared)
        .fallback(ui::serve)
        .with_state(UiDir(ui_dir))
        .layer(axum::middleware::from_fn(browser::guard))
}

/// The largest upload accepted: a machine backup or a sample cut photo.
const UPLOAD_LIMIT: usize = 16 * 1024 * 1024;

/// An API failure as a response.
pub struct Failure(Error);

impl IntoResponse for Failure {
    fn into_response(self) -> Response {
        let status = match self.0 {
            Error::Request(_) => StatusCode::BAD_REQUEST,
            Error::Refused(_) => StatusCode::CONFLICT,
            Error::Missing(_) => StatusCode::NOT_FOUND,
        };
        (status, Json(json!({ "error": self.0.to_string() }))).into_response()
    }
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self(error)
    }
}

type Reply = std::result::Result<Json<Value>, Failure>;

fn ok() -> Json<Value> {
    Json(json!({ "ok": true }))
}

async fn state(State(shared): State<Shared>) -> Json<Value> {
    Json(full(&shared.lock().await.document()))
}

async fn machine_settings(
    State(shared): State<Shared>,
) -> std::result::Result<Json<crate::parameter_settings::View>, Failure> {
    Ok(Json(crate::parameter_settings::view(&shared).await?))
}

async fn save_machine_settings(
    State(shared): State<Shared>,
    Json(change): Json<crate::parameter_settings::Change>,
) -> Reply {
    crate::parameter_settings::save(&shared, change).await?;
    Ok(ok())
}

async fn read_machine_settings(State(shared): State<Shared>) -> Reply {
    machine::parameters(&shared, false, None).await?;
    Ok(ok())
}

#[derive(Deserialize)]
struct SettingsWrite {
    expected: String,
}

async fn write_machine_settings(
    State(shared): State<Shared>,
    Json(change): Json<SettingsWrite>,
) -> Reply {
    machine::parameters(&shared, true, Some(&change.expected)).await?;
    Ok(ok())
}

async fn events(
    State(shared): State<Shared>,
) -> Sse<impl tokio_stream::Stream<Item = std::result::Result<Event, std::convert::Infallible>>> {
    use tokio_stream::StreamExt as _;
    let documents = shared.lock().await.subscribe();
    // The big sections go out only when their revision moved.
    // The first event carries every section, whatever the revisions say,
    // so a page that reconnects after a restart starts from the truth.
    let mut sent = None;
    let closed = tokio_stream::wrappers::WatchStream::new(shared.closing.subscribe())
        .filter(|closed| *closed)
        .map(|_| None);
    let stream = tokio_stream::wrappers::WatchStream::new(documents)
        .map(Some)
        .merge(closed)
        .take_while(Option::is_some)
        .filter_map(|document| document)
        .map(move |document| {
            let patch = crate::document::Patch { document: &document, previous: sent };
            let event = match Event::default().json_data(patch) {
                Ok(event) => {
                    sent = Some(document.revisions);
                    event
                }
                Err(error) => {
                    tracing::error!(%error, "state serialization");
                    Event::default().event("error").data("state serialization failed")
                }
            };
            Ok(event)
        });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn full(document: &Document) -> Value {
    serde_json::to_value(document).unwrap_or(Value::Null)
}

// Library -----------------------------------------------------------------------

#[derive(Deserialize)]
struct Named {
    name: String,
}

#[derive(Deserialize)]
struct FileImport {
    name: String,
    expected: Option<String>,
}

/// A drawing file to import, and how.
#[derive(Deserialize)]
struct PartImport {
    name: String,
    /// JSON [`crate::imports::ImportOptions`].
    options: Option<String>,
}

/// Parses an uploaded drawing off the lock.
async fn parse_part(
    shared: &Shared,
    request: &PartImport,
    body: axum::body::Bytes,
) -> Result<crate::imports::Import, Error> {
    let options = crate::imports::options(request.options.as_deref())?;
    let name = request.name.clone();
    let fonts = shared.lock().await.fonts.fonts.clone();
    tokio::task::spawn_blocking(move || crate::imports::part(&name, &body, &fonts, &options))
        .await
        .map_err(|e| Error::Refused(format!("import task: {e}")))?
}

async fn import_part(
    State(shared): State<Shared>,
    Query(request): Query<PartImport>,
    body: axum::body::Bytes,
) -> Reply {
    let import = parse_part(&shared, &request, body.clone()).await?;
    crate::imports::require_geometry(&import)?;
    let warnings = import.notices();
    let part = shared.lock().await.import_drawing(&request.name, &body, import)?;
    Ok(Json(json!({ "ok": true, "id": part.id, "warnings": warnings })))
}

/// What importing a file would make, without keeping it.
async fn review_part(
    State(shared): State<Shared>,
    Query(request): Query<PartImport>,
    body: axum::body::Bytes,
) -> Reply {
    let import = parse_part(&shared, &request, body).await?;
    let bed = shared
        .lock()
        .await
        .extent()
        .map(|[x, y]| [x[1] - x[0], y[1] - y[0]])
        .filter(|[w, h]| *w > 0. && *h > 0.);
    let name = request.name.clone();
    let review = tokio::task::spawn_blocking(move || crate::imports::review(&name, &import, bed))
        .await
        .map_err(|e| Error::Refused(format!("import task: {e}")))?;
    Ok(Json(serde_json::to_value(review).map_err(|e| Error::Refused(e.to_string()))?))
}

async fn fonts(State(shared): State<Shared>) -> Reply {
    Ok(Json(json!({ "fonts": shared.lock().await.fonts.fonts.faces() })))
}

async fn import_font(
    State(shared): State<Shared>,
    Query(named): Query<Named>,
    body: axum::body::Bytes,
) -> Reply {
    // A disconnected browser must not abandon a durable font commit before
    // publishing it, or let a competing upload overwrite its saved index.
    let (faces, existing) = tokio::spawn(async move {
        let _upload = shared.font_import.lock().await;
        let store = shared.lock().await.fonts.clone();
        let (store, faces, existing) =
            tokio::task::spawn_blocking(move || store.import(&named.name, &body))
                .await
                .map_err(|e| Error::Refused(format!("font import task: {e}")))??;
        shared.lock().await.fonts = store;
        Ok::<_, Error>((faces, existing))
    })
    .await
    .map_err(|e| Error::Refused(format!("font import task: {e}")))??;
    Ok(Json(json!({ "ok": true, "fonts": faces, "existing": existing })))
}

#[derive(Deserialize)]
struct TextInput {
    #[serde(flatten)]
    text: openlaser_svg::Text,
    font: Option<String>,
}

impl TextInput {
    fn svg(&self, fonts: &openlaser_svg::Fonts) -> openlaser_svg::Result<String> {
        self.font
            .as_deref()
            .map_or_else(|| self.text.svg(), |id| self.text.svg_with_font(fonts.face(id)?))
    }
}

async fn preview_text(State(shared): State<Shared>, Json(text): Json<TextInput>) -> Reply {
    let fonts = shared.lock().await.fonts.fonts.clone();
    let imported = tokio::task::spawn_blocking(move || {
        openlaser_svg::import_with_fonts(text.svg(&fonts)?.as_bytes(), &fonts)
    })
    .await
    .map_err(|e| Error::Refused(format!("text task: {e}")))?
    .map_err(|e| Error::Request(e.to_string()))?;
    let bounds =
        imported.drawing.bounds().ok_or_else(|| Error::Request("text has no outlines".into()))?;
    Ok(Json(json!({
        "outline": crate::draft::outline(&imported.drawing, 128),
        "width": bounds.width(), "height": bounds.height(),
        "contours": imported.drawing.contours.len(), "warnings": imported.warnings
    })))
}

#[derive(Deserialize)]
struct NewText {
    name: String,
    text: TextInput,
}

async fn import_text(State(shared): State<Shared>, Json(request): Json<NewText>) -> Reply {
    let fonts = shared.lock().await.fonts.fonts.clone();
    let (source, imported) = tokio::task::spawn_blocking(move || {
        let source = request.text.svg(&fonts)?;
        let imported = openlaser_svg::import_with_fonts(source.as_bytes(), &fonts)?;
        Ok::<_, openlaser_svg::Error>((source, imported))
    })
    .await
    .map_err(|e| Error::Refused(format!("text task: {e}")))?
    .map_err(|e| Error::Request(e.to_string()))?;
    let warnings = imported.warnings.clone();
    let part = shared.lock().await.import_drawing(
        &format!("{}.svg", request.name),
        source.as_bytes(),
        crate::imports::Import {
            drawing: imported.drawing,
            warnings: imported.warnings,
            ..crate::imports::Import::default()
        },
    )?;
    Ok(Json(json!({ "ok": true, "id": part.id, "warnings": warnings })))
}

async fn update_part(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Json(change): Json<ItemChange>,
) -> Reply {
    shared.lock().await.update_part(&Id::from(id.as_str()), change)?;
    Ok(ok())
}

#[derive(Deserialize)]
struct Thickness {
    thickness_mm: f64,
}

async fn duplicate_recipe(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Json(body): Json<Thickness>,
) -> Reply {
    let copy = shared.lock().await.duplicate_recipe(&Id::from(id.as_str()), body.thickness_mm)?;
    Ok(Json(json!({ "ok": true, "id": copy.id })))
}

async fn duplicate_part(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    let copy = shared.lock().await.duplicate_part(&Id::from(id.as_str()))?;
    Ok(Json(json!({ "ok": true, "id": copy.id })))
}

/// How closely a simplified drawing follows its part, and whether to keep it.
#[derive(Deserialize)]
struct SimplifyRequest {
    /// Millimetres.
    tolerance: f64,
    save: bool,
}

/// Simplifies off the coordinator lock; only saving the result takes it again.
async fn simplify_part(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Json(request): Json<SimplifyRequest>,
) -> Reply {
    let id = Id::from(id.as_str());
    let drawing = shared.lock().await.library.part(&id).map_err(Error::from)?.drawing.clone();
    let result = tokio::task::spawn_blocking(move || {
        openlaser_prep::simplify::simplify(&drawing, request.tolerance).map_err(Error::from)
    })
    .await
    .map_err(|error| Error::Refused(format!("simplify task: {error}")))??;
    let view = shared.lock().await.simplified_part(&id, result, request.save)?;
    Ok(Json(serde_json::to_value(view).map_err(|e| Error::Refused(e.to_string()))?))
}

async fn remove_part(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    shared.lock().await.remove_part(&Id::from(id.as_str()))?;
    Ok(ok())
}

#[derive(Deserialize)]
struct NewFolder {
    name: String,
    parent: Option<Id>,
}

async fn add_folder(State(shared): State<Shared>, Json(new): Json<NewFolder>) -> Reply {
    let id = shared.lock().await.add_folder(&new.name, new.parent)?;
    Ok(Json(json!({ "ok": true, "id": id })))
}

async fn update_folder(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Json(change): Json<ItemChange>,
) -> Reply {
    shared.lock().await.update_folder(&Id::from(id.as_str()), change)?;
    Ok(ok())
}

async fn remove_folder(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    shared.lock().await.remove_folder(&Id::from(id.as_str()))?;
    Ok(ok())
}

async fn add_recipe(State(shared): State<Shared>, Json(new): Json<NewRecipe>) -> Reply {
    let recipe = shared.lock().await.add_recipe(&new)?;
    Ok(Json(json!({ "ok": true, "id": recipe.id })))
}

async fn update_recipe(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Json(change): Json<RecipeChange>,
) -> Reply {
    shared.lock().await.update_recipe(&Id::from(id.as_str()), change)?;
    Ok(ok())
}

async fn remove_recipe(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    shared.lock().await.remove_recipe(&Id::from(id.as_str()))?;
    Ok(ok())
}

async fn import_recipe(
    State(shared): State<Shared>,
    Query(options): Query<RecipeImport>,
    body: axum::body::Bytes,
) -> Reply {
    let (recipe, existing) = shared.lock().await.import_recipe_as(&options, &body)?;
    Ok(Json(json!({ "ok": true, "id": recipe.id, "existing": existing })))
}

async fn preview_recipe(
    State(shared): State<Shared>,
    Query(named): Query<Named>,
    body: axum::body::Bytes,
) -> Reply {
    let preview = shared.lock().await.preview_recipe(&named.name, &body)?;
    Ok(Json(json!({ "ok": true, "preview": preview })))
}

async fn set_photo(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    body: axum::body::Bytes,
) -> Reply {
    shared.lock().await.set_photo(&Id::from(id.as_str()), &body)?;
    Ok(ok())
}

async fn photo(State(shared): State<Shared>, Path(sha256): Path<String>) -> Response {
    match shared.lock().await.photo(&sha256) {
        Ok((media_type, bytes)) => (
            [
                (header::CONTENT_TYPE, media_type),
                (header::CACHE_CONTROL, "max-age=31536000, immutable"),
            ],
            bytes,
        )
            .into_response(),
        Err(error) => Failure(error).into_response(),
    }
}

async fn import_machine_file(
    State(shared): State<Shared>,
    Query(named): Query<FileImport>,
    body: axum::body::Bytes,
) -> Reply {
    machine::import_file_reviewed(&shared, &named.name, &body, named.expected.as_deref()).await?;
    Ok(ok())
}

async fn update_job(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Json(change): Json<ItemChange>,
) -> Reply {
    shared.lock().await.update_job(&Id::from(id.as_str()), change)?;
    Ok(ok())
}

async fn remove_job(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    shared.lock().await.remove_job(&Id::from(id.as_str()))?;
    Ok(ok())
}

// Draft ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct Revision {
    revision: u64,
}

/// A mutation and its preparation own the same revision. The response contains
/// that revision's view, so clients can continue editing without waiting for SSE.
async fn edit(
    shared: &Shared,
    revision: Option<u64>,
    change: impl FnOnce(&mut crate::coordinator::Coordinator) -> crate::Result<Value>,
) -> Reply {
    let (fields, input, draft, revision) = {
        let mut coordinator = shared.lock().await;
        if let Some(revision) = revision {
            coordinator.check_draft(revision)?;
        }
        let fields = change(&mut coordinator)?;
        coordinator.queue_draft();
        let document = coordinator.document();
        (fields, coordinator.preparation()?, document.draft, document.draft_revision)
    };
    let target = shared.clone();
    // Once the edit is accepted, its save and preparation have one lifetime.
    // Dropping the HTTP response cannot leave the working copy busy forever.
    let draft = tokio::spawn(async move {
        crate::workspace::flush(&target).await?;
        let draft = match input {
            Some(input) => machine::prepare_input(&target, input).await.map(Some),
            None => Ok(draft),
        }?;
        if let Some(draft) = draft {
            machine::compile_automatically(&target, draft.revision).await?;
            Ok::<_, Error>(target.lock().await.document().draft)
        } else {
            Ok(None)
        }
    })
    .await
    .map_err(|error| Error::Refused(format!("edit task: {error}")))??;
    let revision = draft.as_ref().map_or(revision, |draft| draft.revision);
    let mut reply = json!({ "ok": true, "draft": draft, "draft_revision": revision });
    if let (Some(reply), Some(fields)) = (reply.as_object_mut(), fields.as_object()) {
        reply.extend(fields.clone());
    }
    Ok(Json(reply))
}

async fn open_part(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    edit(&shared, None, |c| {
        c.open_part(&Id::from(id.as_str()))?;
        Ok(Value::Null)
    })
    .await
}

/// Library parts, in the order a job cuts them.
#[derive(Deserialize)]
struct PartList {
    parts: Vec<Id>,
}

async fn open_parts(State(shared): State<Shared>, Json(list): Json<PartList>) -> Reply {
    edit(&shared, None, |c| {
        c.open_parts(&list.parts)?;
        Ok(Value::Null)
    })
    .await
}

async fn add_parts(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(list): Json<PartList>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.add_parts(&list.parts)?;
        Ok(Value::Null)
    })
    .await
}

async fn open_job(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    edit(&shared, None, |c| {
        c.open_job(&Id::from(id.as_str()))?;
        Ok(Value::Null)
    })
    .await
}

async fn set_recipe(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Query(v): Query<Revision>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.set_recipe(&Id::from(id.as_str()))?;
        Ok(Value::Null)
    })
    .await
}

async fn set_features(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(features): Json<Features>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.set_features(features)?;
        Ok(Value::Null)
    })
    .await
}

async fn copy_features(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Query(v): Query<Revision>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.copy_features(&Id::from(id.as_str()))?;
        Ok(Value::Null)
    })
    .await
}

#[derive(Deserialize)]
struct TransformRequest {
    contours: Vec<usize>,
    /// Applied after what the contours have; absent puts them back.
    matrix: Option<Transform>,
}

async fn transform(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<TransformRequest>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.transform(&request.contours, request.matrix)?;
        Ok(Value::Null)
    })
    .await
}

#[derive(Deserialize)]
struct AddRequest {
    #[serde(default)]
    grouping: openlaser_core::grouping::Grouping,
    /// Locations index the contours in this paste, not the existing sheet.
    #[serde(default)]
    lead_overrides: Vec<openlaser_core::features::LeadOverride>,
    /// Drawing contours, each with where it goes.
    contours: Vec<Added>,
}

#[derive(Deserialize)]
struct Added {
    source: usize,
    transform: Transform,
}

/// Answers with the new contours' indices.
async fn add(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<AddRequest>,
) -> Reply {
    let contours: Vec<_> = request.contours.into_iter().map(|c| (c.source, c.transform)).collect();
    edit(&shared, Some(v.revision), |c| {
        Ok(json!({ "contours": c.add_grouped(&contours, &request.lead_overrides, &request.grouping)? }))
    })
    .await
}

#[derive(Deserialize)]
struct RemoveRequest {
    contours: Vec<usize>,
}

async fn group(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<RemoveRequest>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.set_grouped(&request.contours, true)?;
        Ok(Value::Null)
    })
    .await
}

async fn ungroup(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<RemoveRequest>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.set_grouped(&request.contours, false)?;
        Ok(Value::Null)
    })
    .await
}

async fn remove(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<RemoveRequest>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.remove(&request.contours)?;
        Ok(Value::Null)
    })
    .await
}

async fn undo(State(shared): State<Shared>, Query(v): Query<Revision>) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.undo()?;
        Ok(Value::Null)
    })
    .await
}

async fn redo(State(shared): State<Shared>, Query(v): Query<Revision>) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.redo()?;
        Ok(Value::Null)
    })
    .await
}

#[derive(Deserialize)]
struct PickRequest {
    /// Where the tap landed, in drawing coordinates.
    point: [f64; 2],
    /// How far a contour may be from it, in millimetres.
    tolerance: f64,
    #[serde(default)]
    bridging: bool,
    #[serde(default)]
    features: Option<Features>,
}

/// Answers the pick against exactly the displayed geometry.
async fn pick(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<PickRequest>,
) -> Reply {
    let coordinator = shared.lock().await;
    coordinator.check_draft(v.revision)?;
    let picked = coordinator.pick_staged(
        request.point,
        request.tolerance,
        request.bridging,
        request.features,
    )?;
    Ok(Json(json!({ "ok": true, "pick": picked, "revision": v.revision })))
}

/// Pure preview for a staged canvas edit; no history or active draft is written.
async fn preview_features(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(features): Json<Features>,
) -> Reply {
    let (drawing, placed) = {
        let c = shared.lock().await;
        c.check_draft(v.revision)?;
        let d = c.draft.as_ref().ok_or_else(|| Error::Refused("open a part first".into()))?;
        (d.drawing()?.clone(), d.placed.clone())
    };
    let prepared =
        tokio::task::spawn_blocking(move || crate::draft::prepare(&drawing, &placed, &features))
            .await
            .map_err(|e| Error::Refused(e.to_string()))??;
    Ok(Json(json!({ "preview": prepared.preview, "revision": v.revision })))
}

#[derive(Deserialize)]
struct OriginRequest {
    /// Machine coordinates; absent uses the stationary head.
    origin: Option<[f64; 2]>,
    /// Optional anchor chosen together with the origin.
    anchor: Option<openlaser_library::Anchor>,
}

#[derive(Deserialize)]
struct AnchorRequest {
    anchor: openlaser_library::Anchor,
}

async fn set_anchor(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<AnchorRequest>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.set_anchor(request.anchor)?;
        Ok(Value::Null)
    })
    .await
}

async fn set_origin(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<OriginRequest>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        match request.origin {
            Some(origin) => c.set_origin_at(origin, request.anchor)?,
            None => c.origin_here_at(request.anchor)?,
        }
        Ok(Value::Null)
    })
    .await
}

async fn set_placement(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(change): Json<crate::placement::PlacementChange>,
) -> Reply {
    let work = shared.clone();
    tokio::spawn(async move { crate::placement::change(&work, change, v.revision).await })
        .await
        .map_err(|error| Error::Refused(format!("positioning task: {error}")))??;
    let doc = shared.lock().await.document();
    Ok(Json(json!({ "ok": true, "draft": doc.draft, "draft_revision": doc.draft_revision })))
}

async fn prepare_placement(State(shared): State<Shared>, Query(v): Query<Revision>) -> Reply {
    let work = shared.clone();
    tokio::spawn(async move { crate::placement::prepare_revision(&work, Some(v.revision)).await })
        .await
        .map_err(|error| Error::Refused(format!("positioning task: {error}")))??;
    let doc = shared.lock().await.document();
    Ok(Json(json!({ "ok": true, "draft": doc.draft, "draft_revision": doc.draft_revision })))
}

#[derive(Deserialize, Default)]
struct CompileRequest {
    #[serde(default)]
    dry_run: bool,
}

async fn compile(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<CompileRequest>,
) -> Reply {
    machine::compile_revision(&shared, request.dry_run, v.revision).await?;
    let doc = shared.lock().await.document();
    Ok(Json(json!({ "ok": true, "draft": doc.draft, "draft_revision": doc.draft_revision })))
}

async fn save_job(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(named): Json<Named>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| Ok(json!({ "id": c.save_job(&named.name)?.id }))).await
}

async fn set_stock(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(choice): Json<crate::nesting::StockChoice>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.set_stock(choice)?;
        Ok(Value::Null)
    })
    .await
}

async fn nest_start(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<crate::nesting::NestRequest>,
) -> Reply {
    Ok(Json(
        serde_json::to_value(crate::nesting::start(&shared, v.revision, request).await?)
            .map_err(|e| Error::Refused(e.to_string()))?,
    ))
}

/// The live arrangement a caller already has.
#[derive(Deserialize)]
struct LiveSince {
    since: Option<u64>,
}

async fn nest_status(
    State(shared): State<Shared>,
    Path(id): Path<u64>,
    Query(known): Query<LiveSince>,
) -> Reply {
    let mut view = crate::nesting::status(&shared, id).await?;
    if known.since.is_some_and(|since| since >= view.live_serial) {
        view.live = None;
    }
    Ok(Json(serde_json::to_value(view).map_err(|e| Error::Refused(e.to_string()))?))
}

async fn nest_cancel(State(shared): State<Shared>, Path(id): Path<u64>) -> Reply {
    crate::nesting::cancel(&shared, id).await?;
    Ok(ok())
}

async fn nest_apply(State(shared): State<Shared>, Path(id): Path<u64>) -> Reply {
    crate::nesting::apply(&shared, id).await?;
    let document = shared.lock().await.document();
    Ok(Json(json!({ "ok":true,"draft":document.draft,"draft_revision":document.draft_revision })))
}

// Machine -------------------------------------------------------------------------

async fn set_route(State(shared): State<Shared>, Json(change): Json<RouteChange>) -> Reply {
    connect::set_route(&shared, change).await?;
    Ok(ok())
}

// Operator checks and read-only history -----------------------------------------

#[derive(Deserialize)]
struct HistoryQuery {
    before: Option<String>,
}

async fn alarm_history(State(shared): State<Shared>, Query(query): Query<HistoryQuery>) -> Reply {
    let history = shared.lock().await.history.clone();
    Ok(Json(
        serde_json::to_value(history.page(query.before).await?)
            .map_err(|e| Error::Refused(e.to_string()))?,
    ))
}

#[derive(Deserialize)]
struct PreflightQuery {
    intent: crate::preflight::PreflightIntent,
}

async fn preflight_review(
    State(shared): State<Shared>,
    Query(query): Query<PreflightQuery>,
) -> Reply {
    let review = crate::preflight::review(&shared, query.intent).await?;
    Ok(Json(serde_json::to_value(review).map_err(|e| Error::Refused(e.to_string()))?))
}

async fn preflight_preferences(
    State(shared): State<Shared>,
) -> Json<crate::preflight::PreflightPreferences> {
    Json(shared.lock().await.preflight.clone())
}

async fn save_preflight_preferences(
    State(shared): State<Shared>,
    Json(preferences): Json<crate::preflight::PreflightPreferences>,
) -> Reply {
    shared.lock().await.save_preflight_preferences(preferences)?;
    Ok(ok())
}

/// New hold times for every screen, from the ones the caller last read.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TouchChange {
    hold: crate::touch::HoldTimes,
    expected: crate::touch::HoldTimes,
}

async fn save_touch(State(shared): State<Shared>, Json(change): Json<TouchChange>) -> Reply {
    shared.lock().await.save_hold_times(change.hold, change.expected)?;
    Ok(ok())
}

async fn set_preflight(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(policy): Json<openlaser_library::preflight::JobPreflight>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.set_preflight(policy)?;
        Ok(Value::Null)
    })
    .await
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PreflightAction {
    token: String,
    step: usize,
}

async fn preflight_action(
    State(shared): State<Shared>,
    Json(action): Json<PreflightAction>,
) -> Reply {
    crate::preflight::action(&shared, &action.token, action.step).await?;
    Ok(ok())
}

async fn postflight_review(
    State(shared): State<Shared>,
) -> Json<Option<crate::postflight::PostflightReview>> {
    Json(crate::postflight::review(&shared).await)
}

async fn postflight_action(
    State(shared): State<Shared>,
    Json(action): Json<PreflightAction>,
) -> Reply {
    crate::postflight::action(&shared, &action.token, action.step).await?;
    Ok(ok())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PostflightDismiss {
    token: String,
}

async fn dismiss_postflight(
    State(shared): State<Shared>,
    Json(request): Json<PostflightDismiss>,
) -> Reply {
    crate::postflight::dismiss(&shared, &request.token).await?;
    Ok(ok())
}

async fn import_soft(
    State(shared): State<Shared>,
    Query(named): Query<FileImport>,
    body: axum::body::Bytes,
) -> Reply {
    machine::import_soft_reviewed(&shared, &named.name, &body, named.expected.as_deref()).await?;
    Ok(ok())
}

async fn download_soft(State(shared): State<Shared>) -> std::result::Result<Response, Failure> {
    let bytes = shared
        .lock()
        .await
        .soft
        .bytes()
        .ok_or_else(|| Error::Missing("no Soft INI imported".into()))?
        .to_vec();
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (header::CONTENT_DISPOSITION, "attachment; filename=ipAdd.ini"),
        ],
        bytes,
    )
        .into_response())
}

async fn edit_history(
    State(shared): State<Shared>,
) -> std::result::Result<Json<openlaser_library::history::EditHistory>, Failure> {
    Ok(Json(shared.lock().await.library.edit_history().map_err(Error::from)?))
}

async fn undo_saved(
    State(shared): State<Shared>,
    Path(action): Path<String>,
    Json(v): Json<Revision>,
) -> Reply {
    if !matches!(action.as_str(), "undo" | "redo") {
        return Err(Error::Request("unknown history action".into()).into());
    }
    shared.lock().await.undo_saved(action == "undo", v.revision)?;
    Ok(ok())
}

async fn duplicate_job(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    let copy = shared.lock().await.duplicate_job(&Id::from(id.as_str()))?;
    Ok(Json(json!({ "ok": true, "id": copy.id })))
}

async fn pending_drafts(
    State(shared): State<Shared>,
) -> std::result::Result<Json<Vec<crate::workspace::PendingDraft>>, Failure> {
    Ok(Json(shared.lock().await.pending_drafts()?))
}
async fn discard_draft(State(shared): State<Shared>, Path(key): Path<String>) -> Reply {
    crate::workspace::discard(&shared, &key).await?;
    edit(&shared, None, |_| Ok(Value::Null)).await
}

async fn open_retained(State(shared): State<Shared>, Path(key): Path<String>) -> Reply {
    edit(&shared, None, |c| {
        c.open_retained(&key)?;
        Ok(Value::Null)
    })
    .await
}
#[derive(Deserialize)]
struct MergeName {
    name: Option<String>,
}
async fn merge_review(
    State(shared): State<Shared>,
    Query(q): Query<MergeName>,
) -> std::result::Result<Json<crate::workspace::MergeReview>, Failure> {
    Ok(Json(shared.lock().await.merge_review(q.name.as_deref())?))
}
#[derive(Deserialize)]
struct MergeChoices {
    token: String,
    name: Option<String>,
    choices: std::collections::BTreeMap<String, bool>,
}
async fn resolve_merge(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(r): Json<MergeChoices>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        Ok(json!({ "name": c.resolve_merge(&r.token, r.name.as_deref(), &r.choices)? }))
    })
    .await
}

async fn recovery_change(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(change): Json<crate::resume::RecoveryChange>,
) -> Reply {
    shared.lock().await.change_recovery(v.revision, change)?;
    Ok(Json(json!({ "ok": true })))
}
async fn recovery_program(State(shared): State<Shared>) -> Reply {
    let c = shared.lock().await;
    let recovery = c.recovery.as_ref().ok_or_else(|| Error::Refused("no retained job".into()))?;
    Ok(Json(json!({ "execution": recovery.execution })))
}
#[derive(Deserialize)]
struct RecoveryPrepare {
    clearance: bool,
}
async fn recovery_prepare(
    State(shared): State<Shared>,
    Query(v): Query<Revision>,
    Json(request): Json<RecoveryPrepare>,
) -> Reply {
    shared.lock().await.prepare_recovery(v.revision, request.clearance)?;
    Ok(Json(json!({ "ok": true })))
}
async fn recovery_move(State(shared): State<Shared>, Query(v): Query<Revision>) -> Reply {
    machine::move_restart(&shared, v.revision).await?;
    Ok(Json(json!({ "ok": true })))
}

#[derive(Deserialize)]
struct CorrectionMode {
    mode: openlaser_core::LaserMode,
}

async fn correction_view(
    State(shared): State<Shared>,
    Query(query): Query<CorrectionMode>,
) -> Reply {
    let view = shared.lock().await.correction_view(query.mode)?;
    Ok(Json(serde_json::to_value(view).map_err(|e| Error::Refused(e.to_string()))?))
}

async fn correction_save(
    State(shared): State<Shared>,
    Json(change): Json<crate::correction::CorrectionChange>,
) -> Reply {
    shared.lock().await.save_correction(change)?;
    Ok(ok())
}

async fn correction_coupon(
    State(shared): State<Shared>,
    Json(choice): Json<CorrectionMode>,
) -> Reply {
    edit(&shared, None, |c| {
        c.correction_coupon(choice.mode)?;
        Ok(Value::Null)
    })
    .await
}

async fn select_sheet(
    State(shared): State<Shared>,
    Path(index): Path<usize>,
    Query(v): Query<Revision>,
) -> Reply {
    edit(&shared, Some(v.revision), |c| {
        c.select_sheet(index)?;
        Ok(Value::Null)
    })
    .await
}
async fn nest_sheet(State(shared): State<Shared>, Path((id, index)): Path<(u64, usize)>) -> Reply {
    let preview = crate::nesting::sheet_preview(&shared, id, index).await?;
    Ok(Json(serde_json::to_value(preview).map_err(|e| Error::Refused(e.to_string()))?))
}

#[derive(Deserialize)]
struct SheetQuery {
    before: Option<String>,
    #[serde(default)]
    remnants: bool,
}
async fn sheet_history(State(shared): State<Shared>, Query(query): Query<SheetQuery>) -> Reply {
    let page = shared.lock().await.sheet_store.page(query.before.as_deref(), query.remnants)?;
    Ok(Json(serde_json::to_value(page).map_err(|e| Error::Refused(e.to_string()))?))
}
async fn sheet_view(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    let view = shared.lock().await.sheet_store.view(&id)?;
    Ok(Json(serde_json::to_value(view).map_err(|e| Error::Refused(e.to_string()))?))
}
async fn save_remnant(
    State(shared): State<Shared>,
    Path(id): Path<String>,
    Json(change): Json<crate::stock_store::SaveRemnant>,
) -> Reply {
    shared.lock().await.sheet_store.save(&id, &change)?;
    Ok(ok())
}
async fn report_cut_sheet(State(shared): State<Shared>, Path(id): Path<String>) -> Reply {
    let id = shared.lock().await.report_cut_sheet(&Id::from(id.as_str()))?;
    Ok(Json(json!({ "ok": true, "id": id })))
}
