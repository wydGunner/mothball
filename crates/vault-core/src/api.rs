//! Single command dispatcher shared by the Tauri shell and the dev server.
//! `dispatch("files.list", {"projectId": "...", ...})` → JSON result or an
//! error string meant to be shown to the user.

use crate::db::DbResult;
use crate::json::Json;
use crate::projects::NewProject;
use crate::{backup, deploy, deps, files, git, knowledge, printers, profiles, projects, search, snapshots, storage, symbols, tasks, timeline, versions, watch, time, Vault};
use std::path::PathBuf;

fn s(a: &Json, k: &str) -> DbResult<String> {
    a.str_of(k).ok_or_else(|| format!("missing argument: {k}"))
}
fn os(a: &Json, k: &str) -> Option<String> {
    a.str_of(k)
}
fn strs(a: &Json, k: &str) -> Vec<String> {
    a.get(k).and_then(|x| x.as_arr()).map(|v| v.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

pub fn dispatch(v: &Vault, cmd: &str, a: &Json) -> DbResult<Json> {
    let ok = || Ok(Json::obj().with("ok", true));
    match cmd {
        "app.info" => Ok(Json::obj()
            .with("version", crate::APP_VERSION)
            .with("schemaVersion", crate::schema::latest_version())
            .with("dataDir", v.data_dir.to_string_lossy().to_string())
            .with("sqlite", crate::db::sqlite_version())
            .with("categories", Json::Arr(projects::CATEGORIES.iter().map(|c| Json::from(*c)).collect()))
            .with("noteCategories", Json::Arr(knowledge::NOTE_CATEGORIES.iter().map(|c| Json::from(*c)).collect()))),

        // projects
        "deps.list" => deps::list(v, &s(a, "projectId")?),
        "deps.add" => deps::add(v, &s(a, "projectId")?, a),
        "deps.update" => deps::update(v, &s(a, "projectId")?, &s(a, "id")?, a.get("patch").unwrap_or(&Json::obj())),
        "deps.remove" => deps::remove(v, &s(a, "projectId")?, &s(a, "id")?),
        "deps.install" => deps::install(v, &s(a, "projectId")?, os(a, "id").as_deref()),
        "deps.restoreOriginal" => deps::restore_original(v, &s(a, "projectId")?, &s(a, "id")?),
        "printers.list" => printers::list(v),
        "printers.raw" => printers::raw(v, &s(a, "id")?),
        "printers.found" => printers::found_json(v),
        "printers.save" => printers::save(v, a),
        "printers.remove" => printers::remove(v, &s(a, "id")?),
        "printers.resetCertificate" => printers::reset_certificate(v, &s(a, "id")?),
        "printers.history" => printers::history_json(v, a.get("limit").and_then(|x| x.as_i64()).unwrap_or(50).clamp(1, 300) as usize),
        "printers.assign" => printers::assign(v, &s(a, "key")?, os(a, "projectId").as_deref(), a.get("remember").and_then(|x| x.as_bool()).unwrap_or(false)),
        "projects.list" => projects::list(v),
        "projects.get" => {
            let id = s(a, "id")?;
            projects::touch_opened(v, &id)?;
            projects::get(v, &id)
        }
        "projects.create" if a.str_of("copyFrom").filter(|x| !x.trim().is_empty()).is_some() => storage::create_copy(v, a, &PathBuf::from(s(a, "copyFrom")?)),
        "projects.create" => projects::create(
            v,
            NewProject {
                name: s(a, "name")?,
                description: os(a, "description").unwrap_or_default(),
                category: os(a, "category").unwrap_or_else(|| "general".into()),
                status: os(a, "status").unwrap_or_default(),
                link_path: os(a, "linkPath").filter(|p| !p.trim().is_empty()).map(PathBuf::from),
                module: a.get("module").cloned().unwrap_or(Json::obj()),
            },
        ),
        "projects.update" => projects::update(v, &s(a, "id")?, a.get("patch").unwrap_or(&Json::obj())),
        "projects.thumbnail" => projects::thumbnail(v, &s(a, "id")?),
        "projects.setThumbnail" => projects::set_thumbnail(v, &s(a, "id")?, a.get("thumb").unwrap_or(&Json::obj())),
        "projects.thumbnailCandidates" => projects::thumbnail_candidates(v, &s(a, "id")?),
        "files.picture" => projects::file_picture(v, &s(a, "id")?).map(|x| x.map(Json::from).unwrap_or(Json::Null)),
        "projects.relink" => projects::relink(v, &s(a, "id")?, &PathBuf::from(s(a, "path")?)),
        "projects.remove" => projects::remove(v, &s(a, "id")?, a.get("deleteManagedFiles").and_then(|x| x.as_bool()).unwrap_or(false)),
        "projects.statuses" => projects::statuses(v),
        "tags.list" => projects::all_tags(v),

        // files
        "files.scan" => Ok(files::scan(v, &s(a, "projectId")?)?.json()),
        "files.list" => files::list(v, &s(a, "projectId")?, a),
        "files.dirs" => files::dirs(v, &s(a, "projectId")?),
        "files.get" => files::get(v, &s(a, "id")?),
        "files.preview" => files::preview_text(v, &s(a, "id")?, 256 * 1024),
        "files.image" => files::read_base64(v, &s(a, "id")?, 15 * 1024 * 1024),
        "files.mesh" => files::mesh(v, &s(a, "id")?),
        "symbols.find" => symbols::find(v, &s(a, "projectId")?, &s(a, "name")?, &os(a, "qualifier").unwrap_or_default()),
        "symbols.inFile" => symbols::in_file(v, &s(a, "fileId")?),
        "symbols.search" => symbols::search(v, os(a, "projectId").as_deref(), &s(a, "query")?, a.get("limit").and_then(|x| x.as_i64()).unwrap_or(20)),
        "symbols.toc" => symbols::toc_info(v, &s(a, "fileId")?),
        "symbols.references" => symbols::references(v, &s(a, "projectId")?, &s(a, "name")?, 200),
        "files.threemfInfo" => files::threemf_info(v, &s(a, "id")?),
        "files.import" => files::import_op(
            v,
            &s(a, "projectId")?,
            &strs(a, "paths").into_iter().map(PathBuf::from).collect::<Vec<_>>(),
            &os(a, "destDir").unwrap_or_default(),
            a.get("allowDuplicates").and_then(|x| x.as_bool()).unwrap_or(false),
            os(a, "operation").as_deref() == Some("move"),
        ),
        "projects.useFolder" => projects::use_folder(v, &s(a, "id")?, &PathBuf::from(s(a, "path")?)),
        "files.rename" => files::rename(v, &s(a, "id")?, &s(a, "name")?),
        "files.delete" => files::delete(v, &strs(a, "ids")),
        "files.restoreDeleted" => files::restore_deleted(v, &s(a, "id")?),
        "files.readText" => files::read_text(v, &s(a, "id")?),
        "files.writeText" => files::write_text(
            v,
            &s(a, "id")?,
            &a.str_of("text").unwrap_or_default(),
            &s(a, "expectedSha")?,
            &os(a, "eol").unwrap_or_else(|| "lf".into()),
            a.get("bom").and_then(|x| x.as_bool()).unwrap_or(false),
        ),
        "files.saveDocs" => files::docs_save(v, &s(a, "id")?, a.get("docs").unwrap_or(&Json::obj())),
        "files.docRevisions" => files::doc_revisions(v, &s(a, "id")?),
        "files.reviews" => files::reviews(v, &s(a, "projectId")?),
        "files.resolveReview" => files::resolve_review(v, &s(a, "id")?, a.get("accept").and_then(|x| x.as_bool()).unwrap_or(false)),
        "files.forgetMissing" => files::forget_missing(v, &s(a, "id")?).and_then(|_| ok()),
        "files.markReviewed" => files::mark_reviewed(v, &strs(a, "ids")).and_then(|_| ok()),
        "files.duplicates" => files::duplicates(v, &s(a, "projectId")?),
        "files.groups" => files::groups(v, &s(a, "projectId")?),
        "profiles.groups" => Ok(profiles::groups_json(&s(a, "category")?)),
        "projects.dashboard" => projects::dashboard(v, a.get("limit").and_then(|x| x.as_i64()).unwrap_or(8)),
        "time.summary" => time::summary(v, os(a, "projectId").as_deref(), a.get("since").and_then(|x| x.as_i64()).unwrap_or(0)),
        "time.start" => time::start(v, &s(a, "projectId")?, &os(a, "note").unwrap_or_default()),
        "time.stop" => time::stop(v),
        "time.timer" => time::timer(v),
        "time.add" => time::add(
            v,
            &s(a, "projectId")?,
            a.get("start").and_then(|x| x.as_i64()).ok_or("start is required")?,
            a.get("end").and_then(|x| x.as_i64()).ok_or("end is required")?,
            &os(a, "note").unwrap_or_default(),
            "manual",
        ),
        "time.delete" => time::delete(v, &s(a, "id")?).and_then(|_| ok()),
        "time.setEstimate" => time::set_estimate(v, a.get("on").and_then(|x| x.as_bool()).unwrap_or(true)).and_then(|_| ok()),
        "deploy.get" => deploy::get(v, &s(a, "projectId")?),
        "deploy.set" => deploy::set(v, &s(a, "projectId")?, a.get("patch").unwrap_or(&Json::obj())),
        "deploy.preview" => deploy::preview(v, &s(a, "projectId")?),
        "deploy.run" => deploy::run(v, &s(a, "projectId")?, a.get("force").and_then(|x| x.as_bool()).unwrap_or(false)),
        "deploy.launch" => deploy::launch(v, &s(a, "projectId")?),
        "watch.settings" => watch::settings(v),
        "watch.setSettings" => watch::set_settings(v, a.get("patch").unwrap_or(&Json::obj())),
        "watch.stamp" => watch::stamp(v),
        "profiles.guess" => Ok(match profiles::guess(std::path::Path::new(&s(a, "path")?)) {
            Some((c, why, sub)) => Json::obj().with("category", c).with("reason", why).with("subtype", sub),
            None => Json::Null,
        }),
        "types.custom" => projects::custom_types(v),
        "types.saveCustom" => projects::save_custom_types(v, a.get("types").unwrap_or(&Json::Arr(vec![]))),
        "profiles.info" => Ok(profiles::info_json(&s(a, "category")?)),

        // versions
        "versions.list" => versions::list(v, &s(a, "fileId")?),
        "versions.save" => versions::save_manual(v, &s(a, "fileId")?, &os(a, "note").unwrap_or_default()),
        "versions.compare" => versions::compare(v, &s(a, "fileId")?, &s(a, "old")?, &s(a, "new")?),
        "versions.restore" => versions::restore(v, &s(a, "id")?),
        "versions.delete" => versions::delete_version(v, &s(a, "id")?).and_then(|_| ok()),
        "versions.prune" => versions::prune_impl(
            v,
            &s(a, "projectId")?,
            a.get("keep").and_then(|x| x.as_i64()).unwrap_or(10),
            a.get("includeNamed").and_then(|x| x.as_bool()).unwrap_or(false),
            a.get("dryRun").and_then(|x| x.as_bool()).unwrap_or(false),
        ),
        "versions.restoreOpts" => versions::restore_opts(v, &s(a, "id")?, a.get("keepCurrent").and_then(|x| x.as_bool()).unwrap_or(true)),
        "versions.backupNow" => versions::backup_now(v, &s(a, "fileId")?, &os(a, "name").unwrap_or_default(), &os(a, "description").unwrap_or_default(), &os(a, "tags").unwrap_or_default()),
        "versions.suggestName" => versions::suggest_name(v, &s(a, "fileId")?, &os(a, "description").unwrap_or_default(), &os(a, "taskTitle").unwrap_or_default()),
        "versions.update" => versions::update_backup(v, &s(a, "id")?, &os(a, "name").unwrap_or_default(), &os(a, "description").unwrap_or_default(), &os(a, "tags").unwrap_or_default()).and_then(|_| ok()),
        "versions.export" => versions::export_copy(v, &s(a, "id")?, &PathBuf::from(s(a, "dest")?)),
        "versions.browse" => versions::browse(
            v,
            os(a, "projectId").as_deref(),
            &os(a, "query").unwrap_or_default(),
            a.get("namedOnly").and_then(|x| x.as_bool()).unwrap_or(false),
            a.get("from").and_then(|x| x.as_i64()),
            a.get("to").and_then(|x| x.as_i64()),
            a.get("limit").and_then(|x| x.as_i64()).unwrap_or(500),
        ),
        "versions.history" => versions::history(v, &s(a, "fileId")?),
        "versions.smartSave" => versions::smart_save(
            v,
            &s(a, "id")?,
            &a.str_of("text").unwrap_or_default(),
            &s(a, "expectedSha")?,
            &os(a, "eol").unwrap_or_else(|| "lf".into()),
            a.get("bom").and_then(|x| x.as_bool()).unwrap_or(false),
            &versions::SmartSave {
                backup_before: a.get("backupBefore").and_then(|x| x.as_bool()).unwrap_or(false),
                backup_after: a.get("backupAfter").and_then(|x| x.as_bool()).unwrap_or(false),
                name: &os(a, "name").unwrap_or_default(),
                description: &os(a, "description").unwrap_or_default(),
                tags: &os(a, "tags").unwrap_or_default(),
            },
        ),
        "storage.projects" => storage::info(v),
        "storage.setProjectsRoot" => storage::set_root(v, &PathBuf::from(s(a, "path")?), a.get("moveExisting").and_then(|x| x.as_bool()).unwrap_or(true)),
        "storage.get" => Ok(Json::obj().with("root", projects::display_path(&versions::storage_root(v))).with("available", versions::storage_root(v).is_dir()).with("stats", versions::stats(v, None)?)),
        "storage.set" => versions::set_storage_root(v, &PathBuf::from(s(a, "path")?)),
        "snapshots.create" => snapshots::create(v, &s(a, "projectId")?, &os(a, "name").unwrap_or_default(), &os(a, "description").unwrap_or_default(), a.get("maxFileMb").and_then(|x| x.as_f64()).unwrap_or(200.0)),
        "snapshots.list" => snapshots::list(v, &s(a, "projectId")?),
        "snapshots.preview" => snapshots::preview(v, &s(a, "id")?),
        "snapshots.restore" => snapshots::restore(v, &s(a, "id")?),
        "snapshots.delete" => snapshots::delete(v, &s(a, "id")?).and_then(|_| ok()),
        "files.stopTracking" => files::stop_tracking(v, &strs(a, "ids"), a.get("keepVersions").and_then(|x| x.as_bool()).unwrap_or(false)),
        "files.importPreview" => files::import_preview(v, &s(a, "projectId")?, &strs(a, "paths").into_iter().map(PathBuf::from).collect::<Vec<_>>(), &os(a, "destDir").unwrap_or_default()),
        "files.importBatches" => files::import_batches(v, &s(a, "projectId")?),
        "files.undoImport" => files::undo_import(v, &s(a, "id")?),
        "files.moveToProject" => files::move_to_project(v, &strs(a, "ids"), &s(a, "targetProjectId")?, &os(a, "destDir").unwrap_or_default()),
        "versions.stats" => versions::stats(v, os(a, "projectId").as_deref()),
        "versions.policy" => versions::policy_json(v, &s(a, "projectId")?),
        "versions.setPolicy" => versions::set_policy(v, &s(a, "projectId")?, a.get("policy").unwrap_or(&Json::obj())),
        "files.activity" => files::recent_activity(v, &s(a, "projectId")?, a.get("limit").and_then(|x| x.as_i64()).unwrap_or(50)),

        // notes & links
        "notes.tree" => knowledge::tree(v, &s(a, "projectId")?),
        "notes.get" => knowledge::get(v, &s(a, "id")?),
        "notes.create" => knowledge::create(v, &s(a, "projectId")?, &os(a, "title").unwrap_or_default(), os(a, "parentId"), &os(a, "category").unwrap_or_default(), &os(a, "body").unwrap_or_default()),
        "notes.save" => knowledge::save(v, &s(a, "id")?, a.get("patch").unwrap_or(&Json::obj())),
        "notes.delete" => knowledge::delete(v, &s(a, "id")?).and_then(|_| ok()),
        "notes.trash" => knowledge::trash(v, &s(a, "projectId")?),
        "notes.restore" => knowledge::restore(v, &s(a, "id")?),
        "notes.revisions" => knowledge::revisions(v, &s(a, "id")?),
        "links.add" => knowledge::link(v, &s(a, "aType")?, &s(a, "aId")?, &s(a, "bType")?, &s(a, "bId")?, &os(a, "label").unwrap_or_default()),
        "links.remove" => knowledge::unlink(v, &s(a, "linkId")?).and_then(|_| ok()),
        "links.for" => knowledge::links_for(v, &s(a, "type")?, &s(a, "id")?),

        // tasks
        "tasks.list" => tasks::list(v, &s(a, "projectId")?),
        "tasks.get" => tasks::get(v, &s(a, "id")?),
        "tasks.create" => tasks::create(v, &s(a, "projectId")?, a),
        "tasks.update" => tasks::update(v, &s(a, "id")?, a.get("patch").unwrap_or(&Json::obj())),
        "tasks.delete" => tasks::delete(v, &s(a, "id")?).and_then(|_| ok()),
        "milestones.list" => tasks::milestones(v, &s(a, "projectId")?),
        "milestones.save" => tasks::milestone_save(v, &s(a, "projectId")?, a),

        // timeline
        "timeline.list" => timeline::list(v, &s(a, "projectId")?, a.get("limit").and_then(|x| x.as_i64()).unwrap_or(100), a.get("before").and_then(|x| x.as_i64())),
        "timeline.journal" => {
            let pid = s(a, "projectId")?;
            let summary = s(a, "summary")?;
            if summary.trim().is_empty() {
                return Err("Journal entry is empty".into());
            }
            timeline::add(v, &pid, "journal", summary.trim(), &os(a, "detail").unwrap_or_default(), None, true)?;
            ok()
        }
        "timeline.deleteJournal" => timeline::delete_manual(v, &s(a, "id")?).and_then(|_| ok()),

        // search
        "search" => search::search(v, &s(a, "query")?, os(a, "projectId").as_deref(), &strs(a, "types"), a.get("limit").and_then(|x| x.as_i64()).unwrap_or(50)),
        "search.rebuild" => Ok(Json::obj().with("indexed", search::rebuild(v)?)),

        // git
        "git.info" => git::info(&projects::root_of(v, &s(a, "projectId")?)?),
        "git.status" => git::status(&projects::root_of(v, &s(a, "projectId")?)?),
        "git.log" => git::log(&projects::root_of(v, &s(a, "projectId")?)?, os(a, "path").as_deref(), a.get("limit").and_then(|x| x.as_i64()).unwrap_or(100)),
        "git.diff" => git::diff(&projects::root_of(v, &s(a, "projectId")?)?, os(a, "rev").as_deref(), os(a, "path").as_deref()),

        "git.init" => git::init(&projects::root_of(v, &s(a, "projectId")?)?),
        "git.identity" => git::identity(&projects::root_of(v, &s(a, "projectId")?)?),
        "git.setIdentity" => git::set_identity(&projects::root_of(v, &s(a, "projectId")?)?, &s(a, "name")?, &s(a, "email")?),
        "git.commit" => {
            let pid = s(a, "projectId")?;
            let msg = s(a, "message")?;
            let r = git::commit(&projects::root_of(v, &pid)?, &msg, &strs(a, "paths"))?;
            timeline::add(v, &pid, "git", &format!("Committed: {}", msg.lines().next().unwrap_or("").trim()), "", None, false)?;
            Ok(r)
        }
        "git.push" => {
            let pid = s(a, "projectId")?;
            let r = git::push(&projects::root_of(v, &pid)?)?;
            timeline::add(v, &pid, "git", "Pushed commits to the remote", "", None, false)?;
            Ok(r)
        }
        "git.pull" => git::pull(&projects::root_of(v, &s(a, "projectId")?)?),
        "git.fetch" => git::fetch(&projects::root_of(v, &s(a, "projectId")?)?),
        "git.setRemote" => git::set_remote(&projects::root_of(v, &s(a, "projectId")?)?, &s(a, "url")?),
        "git.ghStatus" => Ok(git::gh_status(&projects::root_of(v, &s(a, "projectId")?)?)),
        "git.ghCreate" => git::gh_create(
            &projects::root_of(v, &s(a, "projectId")?)?,
            &s(a, "name")?,
            a.get("private").and_then(|x| x.as_bool()).unwrap_or(true),
            &os(a, "description").unwrap_or_default(),
        ),

        // backup & import
        "backup.create" => backup::create_opts(
            v,
            &PathBuf::from(s(a, "dest")?),
            os(a, "projectId").as_deref(),
            backup::Opts {
                include_files: a.get("includeFiles").and_then(|x| x.as_bool()).unwrap_or(false),
                max_file_mb: a.get("maxFileMb").and_then(|x| x.as_f64()).unwrap_or(500.0),
                zip: a.get("zip").and_then(|x| x.as_bool()).unwrap_or(false),
                auto: false,
            },
        ),
        "backup.inspect" => backup::inspect(v, &PathBuf::from(s(a, "path")?)),
        "backup.restorePreview" => backup::restore_preview(v, &PathBuf::from(s(a, "path")?), &s(a, "projectId")?),
        "backup.restoreFiles" => backup::restore_files(v, &PathBuf::from(s(a, "path")?), &s(a, "projectId")?, a.get("overwrite").and_then(|x| x.as_bool()).unwrap_or(false)),
        "backup.restoreDatabase" => backup::stage_database_restore(v, &PathBuf::from(s(a, "path")?)),
        "backup.cancelDatabaseRestore" => backup::cancel_database_restore(v),
        "backup.pendingRestore" => Ok(Json::obj().with("pending", v.data_dir.join(backup::PENDING_DB).exists())),
        "backup.schedule" => backup::schedule_get(v),
        "backup.setSchedule" => backup::schedule_set(v, a.get("schedule").unwrap_or(&Json::obj())),
        "backup.runDue" => backup::run_due(v, a.get("force").and_then(|x| x.as_bool()).unwrap_or(false)),
        "backup.verify" => backup::verify(&PathBuf::from(s(a, "path")?)),

        _ => Err(format!("unknown command: {cmd}")),
    }
}

/// String-in/string-out entry point for IPC layers. Always returns a JSON
/// envelope: {"ok":true,"data":...} or {"ok":false,"error":"..."}.
pub fn call(v: &Vault, cmd: &str, args_json: &str) -> String {
    let args = match Json::parse(if args_json.trim().is_empty() { "{}" } else { args_json }) {
        Ok(a) => a,
        Err(e) => return Json::obj().with("ok", false).with("error", format!("bad arguments: {e}")).to_string(),
    };
    match dispatch(v, cmd, &args) {
        Ok(d) => Json::obj().with("ok", true).with("data", d).to_string(),
        Err(e) => Json::obj().with("ok", false).with("error", e).to_string(),
    }
}
