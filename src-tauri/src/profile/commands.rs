//! Application services of this context: thin `#[tauri::command]`s named `profile_*`. Each one calls
//! `Core` through `shared::run` / `shared::run_sync` and maps the result to a DTO.
//!
//! Every change goes through `Core::edit_preferences` (a read, change and write of the whole document
//! under the core's one lock, which a notes read holds too), so a click never overwrites a field that
//! changed elsewhere (a Trends "add", a dismissed company, the model's merge). The edit hands back the
//! document as it is after it, which is what the card shows.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{anyhow, bail, ensure, Context as _, Result};
use crate::engine::pipeline::Cv;
use crate::engine::prefs::{clean_notes, cv_upload, PrefSuggestions, Preferences};
use crate::engine::Core;
use tauri::ipc::Invoke;
use tauri::State;

use super::dto::{AddDto, NotesReadDto, ProfileDto};
use super::rules::{
    add_check, is_profile_file_name, notes_status, parse_job_type, unread_notes_message,
    without_item, AddCheck, Field, NotesStatus, Presence,
};
use crate::shared::{run, run_sync, CommandResult};

type CoreState<'a> = State<'a, Arc<Core>>;

// ------------------------------------------------------------------ services

/// The card around `prefs`: the document an edit just returned, so what is shown is that edit's own result.
fn snapshot_of(core: &Core, prefs: Preferences) -> Result<ProfileDto> {
    // Suggestions are a nicety: without them the form still works.
    let suggestions = core.preference_suggestions().unwrap_or_else(|e| {
        tracing::warn!("preference suggestions: {e:#}");
        PrefSuggestions::default()
    });
    let cv = cv_upload::stored(core)?.map(|(cv, _)| cv);
    Ok(ProfileDto::from_parts(
        &core.profile_path(),
        &prefs,
        &suggestions,
        cv.as_ref(),
        core.can_undo_preferences()?,
    ))
}

fn read_snapshot(core: &Core) -> Result<ProfileDto> {
    snapshot_of(core, core.preferences()?)
}

fn field_named(key: &str) -> Result<Field> {
    Field::parse(key).ok_or_else(|| anyhow!("unknown preference list: {key}"))
}

/// Add `text` to the list `field`; `message` says why nothing was added when the user should know.
async fn add_item(core: &Core, field: Field, text: String) -> Result<AddDto> {
    // The verdict is made on the document as it is when the edit runs, under the core's lock.
    let verdict: Arc<Mutex<Option<AddCheck>>> = Arc::default();
    let slot = verdict.clone();
    let typed = text.clone();
    let prefs = core
        .edit_preferences(move |prefs| {
            let check = add_check(field, prefs, &text);
            let adds = check == AddCheck::Add;
            *slot.lock().unwrap_or_else(|e| e.into_inner()) = Some(check);
            adds && field.add(prefs, &text)
        })
        .await?;
    let check = verdict.lock().unwrap_or_else(|e| e.into_inner()).take();
    let message = match check {
        Some(AddCheck::Refuse(message)) => Some(message),
        // It was to be added but it is not there: say what went wrong rather than nothing.
        Some(AddCheck::Add) if field.presence(&prefs, &typed) == Presence::Absent => {
            Some(field.why_not_added(&prefs, &typed))
        }
        _ => None,
    };
    Ok(AddDto {
        profile: snapshot_of(core, prefs)?,
        message,
    })
}

/// Remove `item` by value (not by position): the list may have changed since the chip was drawn.
async fn remove_item(core: &Core, field: Field, item: String) -> Result<ProfileDto> {
    let prefs = core
        .edit_preferences(move |prefs| {
            let kept = without_item(field.list(prefs), &item);
            let removed = kept.len() != field.list(prefs).len();
            *field.list_mut(prefs) = kept;
            removed
        })
        .await?;
    snapshot_of(core, prefs)
}

async fn set_job_type(core: &Core, job_type: &str) -> Result<ProfileDto> {
    let remote = parse_job_type(job_type).ok_or_else(|| anyhow!("unknown job type: {job_type}"))?;
    let prefs = core
        .edit_preferences(move |prefs| {
            let changed = prefs.remote != remote;
            prefs.remote = remote;
            changed
        })
        .await?;
    snapshot_of(core, prefs)
}

/// Save the notes as the user typed them (cleaned); the model reads them in a second step.
async fn save_notes(core: &Core, notes: &str) -> Result<ProfileDto> {
    let saved = clean_notes(notes);
    let prefs = core
        .edit_preferences(move |prefs| {
            let changed = prefs.notes != saved;
            prefs.notes = saved;
            changed
        })
        .await?;
    snapshot_of(core, prefs)
}

/// The model reads `text` once, so it can fill what the form hides (`Core::read_preference_notes`,
/// which saves the notes first when they changed). Its messages are not shown; anything but a clean
/// read is a muted line, never a blocker. Reads and edits queue in the core.
async fn read_notes(core: &Arc<Core>, text: String) -> Result<NotesReadDto> {
    let line = match core.read_preference_notes(text).await {
        Ok(read) => match notes_status(read) {
            NotesStatus::Idle => None,
            NotesStatus::Failed(line) => Some(line),
        },
        Err(e) => Some(unread_notes_message(&format!("{e:#}"))),
    };
    Ok(NotesReadDto {
        profile: read_snapshot(core)?,
        line,
    })
}

async fn undo(core: &Core) -> Result<ProfileDto> {
    let prefs = match core.undo_preference_edit().await? {
        Some(prev) => prev,
        None => core.preferences()?,
    };
    snapshot_of(core, prefs)
}

async fn import_cv(core: &Core, path: &Path) -> Result<ProfileDto> {
    if !cv_upload::is_supported(path) {
        bail!("Use a .pdf, .md or .txt file.");
    }
    cv_upload::import(core, path).await?;
    read_snapshot(core)
}

fn remove_cv(core: &Core) -> Result<ProfileDto> {
    cv_upload::remove(core)?;
    read_snapshot(core)
}

/// Use `path` as the master profile. It must be a YAML file that reads as a profile.
fn set_profile_file(core: &Core, path: &Path) -> Result<ProfileDto> {
    ensure!(is_profile_file_name(path), "Choose a .yaml or .yml file.");
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    ensure!(!text.trim().is_empty(), "That file is empty.");
    Cv::parse(&text).context("That file is not a valid profile")?;
    core.set_profile_path(path)?;
    tracing::info!("profile: the master profile file was changed");
    read_snapshot(core)
}

// ------------------------------------------------------------------ commands

/// The card: preferences, suggestions, the attached CV, whether there is a change to undo.
#[tauri::command]
pub async fn profile_get(core: CoreState<'_>) -> CommandResult<ProfileDto> {
    let core = core.inner().clone();
    run_sync(move || read_snapshot(&core)).await
}

/// Add an entry to the list `field` (`locations` / `domains` / `skills`).
#[tauri::command]
pub async fn profile_add_item(
    core: CoreState<'_>,
    field: String,
    text: String,
) -> CommandResult<AddDto> {
    let core = core.inner().clone();
    run(async move { add_item(&core, field_named(&field)?, text).await }).await
}

/// Remove an entry from the list `field`, by value.
#[tauri::command]
pub async fn profile_remove_item(
    core: CoreState<'_>,
    field: String,
    item: String,
) -> CommandResult<ProfileDto> {
    let core = core.inner().clone();
    run(async move { remove_item(&core, field_named(&field)?, item).await }).await
}

/// Choose the job type (`any` / `remote` / `hybrid` / `onsite`).
#[tauri::command]
pub async fn profile_set_job_type(
    core: CoreState<'_>,
    job_type: String,
) -> CommandResult<ProfileDto> {
    let core = core.inner().clone();
    run(async move { set_job_type(&core, &job_type).await }).await
}

/// Save the notes ("Anything else").
#[tauri::command]
pub async fn profile_save_notes(core: CoreState<'_>, notes: String) -> CommandResult<ProfileDto> {
    let core = core.inner().clone();
    run(async move { save_notes(&core, &notes).await }).await
}

/// Let the model read the notes in the background; resolves when it is done (up to a minute).
/// The progress of the `prefs_chat` graph goes out as `shared:graph-progress`.
#[tauri::command]
pub async fn profile_read_notes(core: CoreState<'_>, notes: String) -> CommandResult<NotesReadDto> {
    let core = core.inner().clone();
    run(async move { read_notes(&core, notes).await }).await
}

/// Put back the preferences from before the last change.
#[tauri::command]
pub async fn profile_undo(core: CoreState<'_>) -> CommandResult<ProfileDto> {
    let core = core.inner().clone();
    run(async move { undo(&core).await }).await
}

/// Attach the CV at `path` (context for suggestions only).
#[tauri::command]
pub async fn profile_import_cv(core: CoreState<'_>, path: String) -> CommandResult<ProfileDto> {
    let core = core.inner().clone();
    run(async move { import_cv(&core, &PathBuf::from(path)).await }).await
}

/// Forget the attached CV.
#[tauri::command]
pub async fn profile_remove_cv(core: CoreState<'_>) -> CommandResult<ProfileDto> {
    let core = core.inner().clone();
    run_sync(move || remove_cv(&core)).await
}

/// Use another file as the master profile.
#[tauri::command]
pub async fn profile_set_path(core: CoreState<'_>, path: String) -> CommandResult<ProfileDto> {
    let core = core.inner().clone();
    run_sync(move || set_profile_file(&core, &PathBuf::from(path))).await
}

/// This context's commands; `main.rs` routes every `profile_*` call here.
pub fn handler() -> impl Fn(Invoke) -> bool + Send + Sync + 'static {
    tauri::generate_handler![
        profile_get,
        profile_add_item,
        profile_remove_item,
        profile_set_job_type,
        profile_save_notes,
        profile_read_notes,
        profile_undo,
        profile_import_cv,
        profile_remove_cv,
        profile_set_path
    ]
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::engine::prefs::chat::MAX_LIST_ITEMS;
    use crate::engine::{runtime, Core, Paths, Secrets};

    use super::{
        add_item, field_named, remove_item, save_notes, set_job_type, set_profile_file, undo,
    };

    fn core() -> (tempfile::TempDir, Arc<Core>) {
        let dir = tempfile::tempdir().unwrap_or_else(|e| panic!("temp dir: {e}"));
        let core = Core::open_at(Paths::new(dir.path().join("jp")), Secrets::memory())
            .unwrap_or_else(|e| panic!("core: {e:#}"));
        (dir, Arc::new(core))
    }

    fn items(dto: &super::ProfileDto, key: &str) -> Vec<String> {
        dto.fields
            .iter()
            .find(|f| f.key == key)
            .map(|f| f.items.clone())
            .unwrap_or_default()
    }

    #[test]
    fn a_topic_is_added_removed_and_the_change_can_be_undone() {
        let (_dir, core) = core();
        runtime::block_on(async {
            let skills = field_named("skills").unwrap_or_else(|e| panic!("{e:#}"));
            let added = add_item(&core, skills, " Rust ".into())
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(items(&added.profile, "skills"), ["Rust"]);
            assert_eq!(added.message, None);
            assert!(added.profile.can_undo);

            // The same skill again, in other letters: nothing to say, nothing added.
            let again = add_item(&core, skills, "rust".into())
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(items(&again.profile, "skills"), ["Rust"]);
            assert_eq!(again.message, None);

            let removed = remove_item(&core, skills, "RUST".into())
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert!(items(&removed, "skills").is_empty());

            let back = undo(&core).await.unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(items(&back, "skills"), ["Rust"], "the removal is undone");
        });
        assert!(
            core.store.has_preferences().unwrap_or(false),
            "the setup banner follows this"
        );
    }

    #[test]
    fn a_topic_in_a_related_list_or_a_full_list_is_refused_with_the_reason() {
        let (_dir, core) = core();
        runtime::block_on(async {
            let saved = core
                .edit_preferences(|prefs| {
                    prefs.avoid_skills = vec!["PHP".into()];
                    prefs.domains = (0..MAX_LIST_ITEMS).map(|i| format!("d{i}")).collect();
                    true
                })
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(saved.avoid_skills, ["PHP"]);

            let skills = field_named("skills").unwrap_or_else(|e| panic!("{e:#}"));
            let avoided = add_item(&core, skills, "php".into())
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert!(avoided.message.is_some_and(|m| m.contains("avoid list")));
            assert!(items(&avoided.profile, "skills").is_empty());

            let domains = field_named("domains").unwrap_or_else(|e| panic!("{e:#}"));
            let full = add_item(&core, domains, "one more".into())
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert!(full.message.is_some_and(|m| m.contains("can hold up to")));

            // Adding the last entry that fits is not refused.
            let locations = field_named("locations").unwrap_or_else(|e| panic!("{e:#}"));
            let ok = add_item(&core, locations, "Hanoi".into())
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(ok.message, None);
        });
        assert!(
            field_named("roles").is_err(),
            "only the three lists of the card"
        );
    }

    #[test]
    fn the_last_entry_that_fits_is_added_without_a_message() {
        let (_dir, core) = core();
        runtime::block_on(async {
            core.edit_preferences(|prefs| {
                prefs.locations = (0..MAX_LIST_ITEMS - 1).map(|i| format!("l{i}")).collect();
                true
            })
            .await
            .unwrap_or_else(|e| panic!("{e:#}"));
            let locations = field_named("locations").unwrap_or_else(|e| panic!("{e:#}"));
            let last = add_item(&core, locations, "last".into())
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(last.message, None, "the list is full only after this add");
            assert_eq!(items(&last.profile, "locations").len(), MAX_LIST_ITEMS);
        });
    }

    #[test]
    fn the_job_type_and_the_notes_are_saved_as_typed() {
        let (_dir, core) = core();
        runtime::block_on(async {
            let remote = set_job_type(&core, "remote")
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(remote.job_type, "remote");
            assert!(set_job_type(&core, "moon").await.is_err());

            let saved = save_notes(&core, "  Senior roles only \n")
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(
                saved.notes, "Senior roles only",
                "trimmed like the core saves them"
            );
            let cleared = save_notes(&core, "")
                .await
                .unwrap_or_else(|e| panic!("{e:#}"));
            assert_eq!(cleared.notes, "");
        });
    }

    #[test]
    fn only_a_yaml_file_that_reads_as_a_profile_becomes_the_master_profile() {
        let (dir, core) = core();
        let write = |name: &str, text: &str| {
            let path = dir.path().join(name);
            std::fs::write(&path, text).unwrap_or_else(|e| panic!("write: {e}"));
            path
        };
        assert!(
            set_profile_file(&core, &write("p.json", "{}")).is_err(),
            "not YAML by name"
        );
        assert!(
            set_profile_file(&core, &write("empty.yaml", "  \n")).is_err(),
            "nothing to read"
        );
        assert!(
            set_profile_file(&core, &write("bad.yaml", "basics: [unclosed")).is_err(),
            "not valid YAML"
        );
        assert!(set_profile_file(&core, &dir.path().join("missing.yaml")).is_err());
        let default_path = core.profile_path();

        let good = write("good.yaml", "basics:\n  name: Test Person\n");
        let dto = set_profile_file(&core, &good).unwrap_or_else(|e| panic!("{e:#}"));
        assert_eq!(dto.profile_path, good.display().to_string());
        assert_ne!(core.profile_path(), default_path);
    }
}
