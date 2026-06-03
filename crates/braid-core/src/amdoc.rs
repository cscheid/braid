//! Conversion between automerge documents and the hydrated schema types.
//!
//! Two directions:
//!
//! - **hydrate**: read an automerge document into a [`TrackerDoc`].
//! - **reconcile**: given desired hydrated state, mutate the document to
//!   match — touching *only* what differs, so unchanged fields generate no
//!   operations (keeps merges clean) and prose fields are updated via
//!   `update_text` (diff-and-splice, so concurrent edits interleave instead
//!   of clobbering).
//!
//! All functions are generic over automerge's `ReadDoc` / `Transactable`
//! traits so they work both on a bare `Automerge` and inside a samod
//! `DocHandle::with_document` transaction.
//!
//! Shape conventions (see the design doc's "Document schema" section):
//! - `ROOT.metadata`: map of scalars
//! - `ROOT.issues`: map keyed by issue id → issue map
//! - prose fields are automerge `Text` (hydration also tolerates plain
//!   string scalars written by foreign tools)
//! - `labels` is a map-as-set (key presence = membership)
//! - `dependencies` / `comments` are maps keyed by edge key / comment id
//!
//! The container maps (`labels`, `dependencies`, `comments`) are created
//! eagerly when an issue is first reconciled, so that concurrent inserts
//! from different replicas land in the *same* map object instead of racing
//! to create it (concurrent `put_object` of the same key would make one
//! side's container — and its contents — lose).

use automerge::{ObjId, ObjType, ReadDoc, ScalarValue, Value, transaction::Transactable};

use crate::schema::{
    Comment, Dependency, Issue, SCHEMA_VERSION, TrackerDoc, TrackerMetadata,
};

#[derive(Debug, thiserror::Error)]
pub enum HydrateError {
    #[error(transparent)]
    Automerge(#[from] automerge::AutomergeError),
    #[error("document is not an initialized braid tracker: missing {0}")]
    NotATracker(&'static str),
    #[error("unsupported schema_version {found} (this braid supports {supported})")]
    UnsupportedSchemaVersion { found: i64, supported: i64 },
    #[error("unexpected shape at {path}: expected {expected}")]
    Shape { path: String, expected: &'static str },
}

#[derive(Debug, thiserror::Error)]
pub enum ReconcileError {
    #[error(transparent)]
    Automerge(#[from] automerge::AutomergeError),
    #[error("issue map key {key:?} does not match issue id {id:?}")]
    IdMismatch { key: String, id: String },
    #[error("dependency map key {key:?} does not match edge key {expected:?}")]
    DepKeyMismatch { key: String, expected: String },
    #[error("comment map key {key:?} does not match comment id {id:?}")]
    CommentKeyMismatch { key: String, id: String },
}

// ---------------------------------------------------------------------------
// reconcile helpers
// ---------------------------------------------------------------------------

/// Get the object at `parent[key]` if it exists with the right type;
/// otherwise (re)create it.
fn ensure_obj<T: Transactable>(
    tx: &mut T,
    parent: &ObjId,
    key: &str,
    ty: ObjType,
) -> Result<ObjId, automerge::AutomergeError> {
    if let Some((Value::Object(t), id)) = tx.get(parent, key)?
        && t == ty {
            return Ok(id);
        }
    tx.put_object(parent, key, ty)
}

fn current_str<R: ReadDoc>(
    doc: &R,
    obj: &ObjId,
    key: &str,
) -> Result<Option<String>, automerge::AutomergeError> {
    Ok(match doc.get(obj, key)? {
        Some((Value::Scalar(s), _)) => match s.as_ref() {
            ScalarValue::Str(cur) => Some(cur.to_string()),
            _ => None,
        },
        _ => None,
    })
}

fn put_str_if_changed<T: Transactable>(
    tx: &mut T,
    obj: &ObjId,
    key: &str,
    val: &str,
) -> Result<(), automerge::AutomergeError> {
    if current_str(tx, obj, key)?.as_deref() != Some(val) {
        tx.put(obj, key, val)?;
    }
    Ok(())
}

fn put_opt_str_if_changed<T: Transactable>(
    tx: &mut T,
    obj: &ObjId,
    key: &str,
    val: Option<&str>,
) -> Result<(), automerge::AutomergeError> {
    match val {
        Some(v) => put_str_if_changed(tx, obj, key, v),
        None => {
            if tx.get(obj, key)?.is_some() {
                tx.delete(obj, key)?;
            }
            Ok(())
        }
    }
}

fn put_int_if_changed<T: Transactable>(
    tx: &mut T,
    obj: &ObjId,
    key: &str,
    val: i64,
) -> Result<(), automerge::AutomergeError> {
    let unchanged = matches!(
        tx.get(obj, key)?,
        Some((Value::Scalar(s), _)) if matches!(s.as_ref(), ScalarValue::Int(cur) if *cur == val)
    );
    if !unchanged {
        tx.put(obj, key, val)?;
    }
    Ok(())
}

/// Reconcile a prose field stored as automerge `Text`.
///
/// - present + desired: `update_text` only if different (diff-and-splice)
/// - missing/wrong-type + desired: fresh `Text` object
/// - present + not desired: delete the key
fn reconcile_text<T: Transactable>(
    tx: &mut T,
    obj: &ObjId,
    key: &str,
    desired: Option<&str>,
) -> Result<(), automerge::AutomergeError> {
    match (tx.get(obj, key)?, desired) {
        (Some((Value::Object(ObjType::Text), id)), Some(want)) => {
            if tx.text(&id)? != want {
                tx.update_text(&id, want)?;
            }
        }
        (_, Some(want)) => {
            let id = tx.put_object(obj, key, ObjType::Text)?;
            tx.splice_text(&id, 0, 0, want)?;
        }
        (Some(_), None) => tx.delete(obj, key)?,
        (None, None) => {}
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// reconcile
// ---------------------------------------------------------------------------

/// Initialize (or re-assert) the tracker skeleton: `metadata` and the
/// `issues` map. Idempotent: identical metadata generates no operations.
pub fn init_tracker<T: Transactable>(
    tx: &mut T,
    meta: &TrackerMetadata,
) -> Result<(), ReconcileError> {
    let meta_obj = ensure_obj(tx, &automerge::ROOT, "metadata", ObjType::Map)?;
    put_int_if_changed(tx, &meta_obj, "schema_version", meta.schema_version)?;
    put_str_if_changed(tx, &meta_obj, "name", &meta.name)?;
    put_str_if_changed(tx, &meta_obj, "id_prefix", &meta.id_prefix)?;
    put_str_if_changed(tx, &meta_obj, "created_at", &meta.created_at)?;
    ensure_obj(tx, &automerge::ROOT, "issues", ObjType::Map)?;
    Ok(())
}

/// Upsert a single issue (keyed by `issue.id`), touching only fields that
/// differ from the current document state.
pub fn reconcile_issue<T: Transactable>(tx: &mut T, issue: &Issue) -> Result<(), ReconcileError> {
    let issues_obj = ensure_obj(tx, &automerge::ROOT, "issues", ObjType::Map)?;
    let obj = ensure_obj(tx, &issues_obj, &issue.id, ObjType::Map)?;

    put_str_if_changed(tx, &obj, "id", &issue.id)?;
    put_str_if_changed(tx, &obj, "title", &issue.title)?;
    put_str_if_changed(tx, &obj, "status", issue.status.as_str())?;
    put_int_if_changed(tx, &obj, "priority", issue.priority)?;
    put_str_if_changed(tx, &obj, "issue_type", issue.issue_type.as_str())?;
    put_str_if_changed(tx, &obj, "created_at", &issue.created_at)?;
    put_str_if_changed(tx, &obj, "created_by", &issue.created_by)?;
    put_str_if_changed(tx, &obj, "updated_at", &issue.updated_at)?;
    put_opt_str_if_changed(tx, &obj, "assignee", issue.assignee.as_deref())?;
    put_opt_str_if_changed(tx, &obj, "closed_at", issue.closed_at.as_deref())?;
    put_opt_str_if_changed(tx, &obj, "close_reason", issue.close_reason.as_deref())?;
    put_opt_str_if_changed(tx, &obj, "external_ref", issue.external_ref.as_deref())?;

    reconcile_text(tx, &obj, "description", issue.description.as_deref())?;
    reconcile_text(tx, &obj, "design", issue.design.as_deref())?;
    reconcile_text(tx, &obj, "acceptance_criteria", issue.acceptance_criteria.as_deref())?;
    reconcile_text(tx, &obj, "notes", issue.notes.as_deref())?;

    // labels: map-as-set
    let labels_obj = ensure_obj(tx, &obj, "labels", ObjType::Map)?;
    let current: Vec<String> = tx.keys(&labels_obj).collect();
    for k in &current {
        if !issue.labels.contains(k) {
            tx.delete(&labels_obj, k.as_str())?;
        }
    }
    for k in &issue.labels {
        let present = matches!(
            tx.get(&labels_obj, k.as_str())?,
            Some((Value::Scalar(s), _)) if matches!(s.as_ref(), ScalarValue::Boolean(true))
        );
        if !present {
            tx.put(&labels_obj, k.as_str(), true)?;
        }
    }

    // dependencies: map keyed by "<depends_on_id>:<type>"
    let deps_obj = ensure_obj(tx, &obj, "dependencies", ObjType::Map)?;
    for (key, dep) in &issue.dependencies {
        let expected = dep.key();
        if *key != expected {
            return Err(ReconcileError::DepKeyMismatch { key: key.clone(), expected });
        }
    }
    let current: Vec<String> = tx.keys(&deps_obj).collect();
    for k in &current {
        if !issue.dependencies.contains_key(k) {
            tx.delete(&deps_obj, k.as_str())?;
        }
    }
    for (key, dep) in &issue.dependencies {
        let dobj = ensure_obj(tx, &deps_obj, key, ObjType::Map)?;
        put_str_if_changed(tx, &dobj, "depends_on_id", &dep.depends_on_id)?;
        put_str_if_changed(tx, &dobj, "type", dep.dep_type.as_str())?;
        put_str_if_changed(tx, &dobj, "created_at", &dep.created_at)?;
        put_str_if_changed(tx, &dobj, "created_by", &dep.created_by)?;
    }

    // comments: map keyed by comment id
    let comments_obj = ensure_obj(tx, &obj, "comments", ObjType::Map)?;
    for (key, comment) in &issue.comments {
        if *key != comment.id {
            return Err(ReconcileError::CommentKeyMismatch {
                key: key.clone(),
                id: comment.id.clone(),
            });
        }
    }
    let current: Vec<String> = tx.keys(&comments_obj).collect();
    for k in &current {
        if !issue.comments.contains_key(k) {
            tx.delete(&comments_obj, k.as_str())?;
        }
    }
    for (key, comment) in &issue.comments {
        let cobj = ensure_obj(tx, &comments_obj, key, ObjType::Map)?;
        put_str_if_changed(tx, &cobj, "id", &comment.id)?;
        put_str_if_changed(tx, &cobj, "author", &comment.author)?;
        put_str_if_changed(tx, &cobj, "created_at", &comment.created_at)?;
        reconcile_text(tx, &cobj, "text", Some(&comment.text))?;
    }

    Ok(())
}

/// Make the document match `tracker` exactly: reconciles metadata and every
/// issue, and **deletes issues not present in `tracker`**. Full-state sync,
/// meant for import-style flows; day-to-day mutation should use
/// [`reconcile_issue`].
pub fn reconcile_tracker<T: Transactable>(
    tx: &mut T,
    tracker: &TrackerDoc,
) -> Result<(), ReconcileError> {
    for (key, issue) in &tracker.issues {
        if *key != issue.id {
            return Err(ReconcileError::IdMismatch { key: key.clone(), id: issue.id.clone() });
        }
    }
    init_tracker(tx, &tracker.metadata)?;
    let issues_obj = ensure_obj(tx, &automerge::ROOT, "issues", ObjType::Map)?;
    let current: Vec<String> = tx.keys(&issues_obj).collect();
    for k in &current {
        if !tracker.issues.contains_key(k) {
            tx.delete(&issues_obj, k.as_str())?;
        }
    }
    for issue in tracker.issues.values() {
        reconcile_issue(tx, issue)?;
    }
    Ok(())
}

/// Remove an issue from the document. Returns `true` if it was present.
pub fn delete_issue<T: Transactable>(tx: &mut T, id: &str) -> Result<bool, ReconcileError> {
    let Some((Value::Object(ObjType::Map), issues_obj)) = tx.get(automerge::ROOT, "issues")?
    else {
        return Ok(false);
    };
    if tx.get(&issues_obj, id)?.is_some() {
        tx.delete(&issues_obj, id)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// hydrate helpers
// ---------------------------------------------------------------------------

fn shape(path: impl Into<String>, expected: &'static str) -> HydrateError {
    HydrateError::Shape { path: path.into(), expected }
}

fn req_map<R: ReadDoc>(
    doc: &R,
    parent: &ObjId,
    key: &str,
    path: &str,
) -> Result<ObjId, HydrateError> {
    match doc.get(parent, key)? {
        Some((Value::Object(ObjType::Map), id)) => Ok(id),
        _ => Err(shape(path, "map")),
    }
}

fn req_str<R: ReadDoc>(
    doc: &R,
    obj: &ObjId,
    key: &str,
    path: &str,
) -> Result<String, HydrateError> {
    match doc.get(obj, key)? {
        Some((Value::Scalar(s), _)) => match s.as_ref() {
            ScalarValue::Str(v) => Ok(v.to_string()),
            _ => Err(shape(format!("{path}.{key}"), "string")),
        },
        _ => Err(shape(format!("{path}.{key}"), "string")),
    }
}

fn opt_str<R: ReadDoc>(
    doc: &R,
    obj: &ObjId,
    key: &str,
    path: &str,
) -> Result<Option<String>, HydrateError> {
    match doc.get(obj, key)? {
        None => Ok(None),
        Some((Value::Scalar(s), _)) => match s.as_ref() {
            ScalarValue::Str(v) => Ok(Some(v.to_string())),
            ScalarValue::Null => Ok(None),
            _ => Err(shape(format!("{path}.{key}"), "string or absent")),
        },
        Some(_) => Err(shape(format!("{path}.{key}"), "string or absent")),
    }
}

fn req_int<R: ReadDoc>(doc: &R, obj: &ObjId, key: &str, path: &str) -> Result<i64, HydrateError> {
    match doc.get(obj, key)? {
        Some((Value::Scalar(s), _)) => match s.as_ref() {
            ScalarValue::Int(v) => Ok(*v),
            ScalarValue::Uint(v) => Ok(*v as i64),
            _ => Err(shape(format!("{path}.{key}"), "integer")),
        },
        _ => Err(shape(format!("{path}.{key}"), "integer")),
    }
}

/// A prose field: automerge `Text` preferred, plain string scalar tolerated
/// (a foreign writer may have written one).
fn opt_text<R: ReadDoc>(
    doc: &R,
    obj: &ObjId,
    key: &str,
    path: &str,
) -> Result<Option<String>, HydrateError> {
    match doc.get(obj, key)? {
        None => Ok(None),
        Some((Value::Object(ObjType::Text), id)) => Ok(Some(doc.text(&id)?)),
        Some((Value::Scalar(s), _)) => match s.as_ref() {
            ScalarValue::Str(v) => Ok(Some(v.to_string())),
            ScalarValue::Null => Ok(None),
            _ => Err(shape(format!("{path}.{key}"), "text or string")),
        },
        Some(_) => Err(shape(format!("{path}.{key}"), "text or string")),
    }
}

fn req_text<R: ReadDoc>(
    doc: &R,
    obj: &ObjId,
    key: &str,
    path: &str,
) -> Result<String, HydrateError> {
    opt_text(doc, obj, key, path)?.ok_or_else(|| shape(format!("{path}.{key}"), "text"))
}

// ---------------------------------------------------------------------------
// hydrate
// ---------------------------------------------------------------------------

/// Read the whole document into hydrated form.
pub fn hydrate<R: ReadDoc>(doc: &R) -> Result<TrackerDoc, HydrateError> {
    let Some((Value::Object(ObjType::Map), meta_obj)) = doc.get(automerge::ROOT, "metadata")?
    else {
        return Err(HydrateError::NotATracker("metadata"));
    };
    let schema_version = req_int(doc, &meta_obj, "schema_version", "metadata")?;
    if schema_version != SCHEMA_VERSION {
        return Err(HydrateError::UnsupportedSchemaVersion {
            found: schema_version,
            supported: SCHEMA_VERSION,
        });
    }
    let metadata = TrackerMetadata {
        schema_version,
        name: req_str(doc, &meta_obj, "name", "metadata")?,
        id_prefix: req_str(doc, &meta_obj, "id_prefix", "metadata")?,
        created_at: req_str(doc, &meta_obj, "created_at", "metadata")?,
    };

    let Some((Value::Object(ObjType::Map), issues_obj)) = doc.get(automerge::ROOT, "issues")?
    else {
        return Err(HydrateError::NotATracker("issues"));
    };

    let mut issues = std::collections::BTreeMap::new();
    let keys: Vec<String> = doc.keys(&issues_obj).collect();
    for key in keys {
        let path = format!("issues.{key}");
        let obj = req_map(doc, &issues_obj, &key, &path)?;
        let issue = hydrate_issue(doc, &obj, &path)?;
        issues.insert(key, issue);
    }

    Ok(TrackerDoc { metadata, issues })
}

fn hydrate_issue<R: ReadDoc>(doc: &R, obj: &ObjId, path: &str) -> Result<Issue, HydrateError> {
    let mut labels = std::collections::BTreeSet::new();
    if let Some((Value::Object(ObjType::Map), labels_obj)) = doc.get(obj, "labels")? {
        // presence of a key = membership; the stored value is ignored
        labels.extend(doc.keys(&labels_obj));
    }

    let mut dependencies = std::collections::BTreeMap::new();
    if let Some((Value::Object(ObjType::Map), deps_obj)) = doc.get(obj, "dependencies")? {
        let keys: Vec<String> = doc.keys(&deps_obj).collect();
        for key in keys {
            let dpath = format!("{path}.dependencies.{key}");
            let dobj = req_map(doc, &deps_obj, &key, &dpath)?;
            dependencies.insert(
                key,
                Dependency {
                    depends_on_id: req_str(doc, &dobj, "depends_on_id", &dpath)?,
                    dep_type: req_str(doc, &dobj, "type", &dpath)?.as_str().into(),
                    created_at: req_str(doc, &dobj, "created_at", &dpath)?,
                    created_by: req_str(doc, &dobj, "created_by", &dpath)?,
                },
            );
        }
    }

    let mut comments = std::collections::BTreeMap::new();
    if let Some((Value::Object(ObjType::Map), comments_obj)) = doc.get(obj, "comments")? {
        let keys: Vec<String> = doc.keys(&comments_obj).collect();
        for key in keys {
            let cpath = format!("{path}.comments.{key}");
            let cobj = req_map(doc, &comments_obj, &key, &cpath)?;
            comments.insert(
                key,
                Comment {
                    id: req_str(doc, &cobj, "id", &cpath)?,
                    author: req_str(doc, &cobj, "author", &cpath)?,
                    created_at: req_str(doc, &cobj, "created_at", &cpath)?,
                    text: req_text(doc, &cobj, "text", &cpath)?,
                },
            );
        }
    }

    Ok(Issue {
        id: req_str(doc, obj, "id", path)?,
        title: req_str(doc, obj, "title", path)?,
        description: opt_text(doc, obj, "description", path)?,
        design: opt_text(doc, obj, "design", path)?,
        acceptance_criteria: opt_text(doc, obj, "acceptance_criteria", path)?,
        notes: opt_text(doc, obj, "notes", path)?,
        status: req_str(doc, obj, "status", path)?.as_str().into(),
        priority: req_int(doc, obj, "priority", path)?,
        issue_type: req_str(doc, obj, "issue_type", path)?.as_str().into(),
        assignee: opt_str(doc, obj, "assignee", path)?,
        created_at: req_str(doc, obj, "created_at", path)?,
        created_by: req_str(doc, obj, "created_by", path)?,
        updated_at: req_str(doc, obj, "updated_at", path)?,
        closed_at: opt_str(doc, obj, "closed_at", path)?,
        close_reason: opt_str(doc, obj, "close_reason", path)?,
        external_ref: opt_str(doc, obj, "external_ref", path)?,
        labels,
        dependencies,
        comments,
    })
}
