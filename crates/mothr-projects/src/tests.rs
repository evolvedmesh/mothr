use super::*;
use std::fs;
use std::time::Instant;

fn touch(path: &Path) {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).unwrap();
    }
    fs::write(path, "").unwrap();
}

fn git_init(dir: &Path, branch: &str) {
    fs::create_dir_all(dir.join(".git")).unwrap();
    fs::write(dir.join(".git/HEAD"), format!("ref: refs/heads/{branch}\n")).unwrap();
}

/// A small fake filesystem with a variety of projects and things that must be skipped.
fn fixture() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let r = tmp.path();
    git_init(&r.join("mothr"), "main");
    touch(&r.join("mothr/Cargo.toml"));
    touch(&r.join("mothr/crates/inner/package.json")); // inside a git repo: not separate
    git_init(&r.join("other-mothr-tools"), "dev");
    touch(&r.join("other-mothr-tools/Cargo.toml"));
    touch(&r.join("web/app/package.json"));
    touch(&r.join("web/app/tsconfig.json"));
    touch(&r.join("web/app/node_modules/leftpad/package.json")); // skipped
    touch(&r.join("py/pyproject.toml"));
    touch(&r.join("py/svc/go.mod")); // non-git project: descend into it
    touch(&r.join("py/.venv/lib/pyproject.toml")); // hidden: skipped
    touch(&r.join("rusty/target/debug/build/x/Cargo.toml")); // target/: skipped
    touch(&r.join("deep/a/b/c/d/Cargo.toml")); // depth 5: beyond max depth 4
    fs::create_dir_all(r.join("notes")).unwrap(); // no markers
    #[cfg(unix)]
    std::os::unix::fs::symlink(r.join("mothr"), r.join("link-to-mothr")).unwrap();
    tmp
}

fn mem_index() -> ProjectIndex {
    ProjectIndex::with_options(IndexOptions {
        store_path: None,
        seed_from_history: false,
    })
}

fn names(index: &ProjectIndex, root: &Path) -> Vec<String> {
    let mut v: Vec<String> = index
        .search("", 1000)
        .into_iter()
        .map(|h| {
            h.project
                .path
                .strip_prefix(root)
                .unwrap()
                .display()
                .to_string()
        })
        .collect();
    v.sort();
    v
}

#[test]
fn scan_finds_projects_and_skips_noise() {
    let tmp = fixture();
    let index = mem_index();
    let n = index.scan(&[tmp.path().to_path_buf()], 4);
    assert_eq!(n, 5);
    assert_eq!(index.len(), 5);
    assert_eq!(
        names(&index, tmp.path()),
        ["mothr", "other-mothr-tools", "py", "py/svc", "web/app"]
    );

    let shep = index.get(&tmp.path().join("mothr")).unwrap();
    assert_eq!(shep.name, "mothr");
    assert_eq!(shep.git_branch.as_deref(), Some("main"));
    assert_eq!(shep.tags, ["git", "rust"]);
    let web = index.get(&tmp.path().join("web/app")).unwrap();
    assert_eq!(web.tags, ["node", "ts"]);
    assert_eq!(web.git_branch, None);

    // Deeper scan reaches the deep project; overlapping roots don't duplicate.
    let n = index.scan(
        &[
            tmp.path().to_path_buf(),
            tmp.path().join("mothr"),
            tmp.path().join("missing"),
        ],
        6,
    );
    assert_eq!(n, 6);
}

#[test]
fn search_ranks_exact_name_first_with_indices() {
    let tmp = fixture();
    let index = mem_index();
    index.scan(&[tmp.path().to_path_buf()], 4);

    let hits = index.search("mothr", 10);
    assert!(hits.len() >= 2, "{hits:?}");
    assert_eq!(hits[0].project.name, "mothr");
    assert_eq!(hits[1].project.name, "other-mothr-tools");
    assert_eq!(hits[0].name_indices, (0..5).collect::<Vec<u32>>());
    let tilde: Vec<char> = tildify(&hits[0].project.path).chars().collect();
    assert!(!hits[0].path_indices.is_empty());
    let matched: String = hits[0]
        .path_indices
        .iter()
        .map(|&i| tilde[i as usize])
        .collect();
    assert_eq!(matched, "mothr");

    // Fuzzy, smart case, and path-only matches.
    let hits = index.search("mthrtls", 10);
    assert_eq!(hits[0].project.name, "other-mothr-tools");
    assert!(index.search("MOTHR", 10).is_empty());
    let hits = index.search("web app", 10);
    assert_eq!(hits[0].project.name, "app");
    assert!(hits[0].name_indices.len() == 3 && !hits[0].path_indices.is_empty());
    assert!(index.search("zzzqqq", 10).is_empty());
    assert_eq!(index.search("s", 1).len(), 1);
}

#[test]
fn pins_recents_and_empty_query_order() {
    let tmp = fixture();
    let index = mem_index();
    index.scan(&[tmp.path().to_path_buf()], 4);
    let py = tmp.path().join("py");
    let web = tmp.path().join("web/app");
    let gone = tmp.path().join("deleted-project");

    index.set_pinned(&py, true);
    index.set_pinned(&gone, true);
    index.touch_recent(&web);
    index.touch_recent(&py);

    assert_eq!(
        index
            .pinned()
            .iter()
            .map(|p| p.path.clone())
            .collect::<Vec<_>>(),
        [py.clone(), gone.clone()]
    );
    assert!(index.pinned().iter().all(|p| p.pinned));
    assert_eq!(
        index
            .recent(10)
            .iter()
            .map(|p| p.path.clone())
            .collect::<Vec<_>>(),
        [py.clone(), web.clone()]
    );
    assert_eq!(index.recent(1).len(), 1);

    let hits = index.search("  ", 100);
    let order: Vec<&Path> = hits.iter().map(|h| h.project.path.as_path()).collect();
    assert_eq!(&order[..3], [py.as_path(), gone.as_path(), web.as_path()]);
    assert_eq!(hits.len(), 6);
    assert!(hits.windows(2).all(|w| w[0].score > w[1].score));

    index.set_pinned(&py, false);
    assert_eq!(index.pinned().len(), 1);
    assert!(!index.get(&py).unwrap().pinned);

    for i in 0..60 {
        index.touch_recent(&tmp.path().join(format!("nope-{i}")));
    }
    assert_eq!(index.recent(100).len(), MAX_RECENT);
}

#[test]
fn persistence_round_trip_and_manual_projects() {
    let tmp = fixture();
    let store = tmp.path().join("state/sub/projects.json");
    let opts = IndexOptions {
        store_path: Some(store.clone()),
        seed_from_history: false,
    };
    let notes = tmp.path().join("notes");
    {
        let index = ProjectIndex::with_options(opts.clone());
        assert!(index.is_empty());
        index.scan(&[tmp.path().to_path_buf()], 4);
        assert!(index.add_manual(tmp.path().join("missing")).is_none());
        let added = index.add_manual(notes.clone()).unwrap();
        assert_eq!(added.name, "notes");
        index.set_pinned(&tmp.path().join("mothr"), true);
        index.touch_recent(&tmp.path().join("web/app"));
        assert!(store.is_file());
    }
    let index = ProjectIndex::with_options(opts);
    assert_eq!(index.len(), 6);
    assert!(index.get(&notes).is_some());
    assert_eq!(index.pinned()[0].name, "mothr");
    assert_eq!(index.recent(5)[0].name, "app");
    assert_eq!(index.search("mothr", 5)[0].project.name, "mothr");

    // Manual projects survive a rescan of unrelated roots.
    let n = index.scan(&[tmp.path().join("py")], 4);
    assert!(index.get(&notes).is_some());
    // py, py/svc, notes (manual), mothr (pinned), web/app (recent)
    assert_eq!(n, 5);

    // A corrupt store is ignored rather than fatal.
    fs::write(&store, b"{not json").unwrap();
    let index = ProjectIndex::with_options(IndexOptions {
        store_path: Some(store),
        seed_from_history: false,
    });
    assert!(index.is_empty());
}

#[test]
fn search_is_fast_on_many_projects() {
    let index = mem_index();
    {
        let mut st = index.state.write();
        let entries = (0..5000)
            .map(|i| {
                let path = PathBuf::from(format!(
                    "/home/u/projects/group-{}/project-{i}-{}",
                    i % 37,
                    ["api", "web", "cli", "core"][i % 4]
                ));
                Entry::new(ProjectRef::from_path(path), i as i64, false)
            })
            .collect();
        st.set_entries(entries);
    }
    index.search("proj", 20); // warm up
    let t = Instant::now();
    let hits = index.search("grp3 core", 20);
    let elapsed = t.elapsed();
    assert_eq!(hits.len(), 20);
    eprintln!("5000-entry search: {elapsed:?}");
    if !cfg!(debug_assertions) {
        assert!(elapsed.as_millis() < 5, "{elapsed:?}");
    }
}

/// Real-world timing: `cargo test -p mothr-projects --release -- --ignored --nocapture real_world`
#[test]
#[ignore]
fn real_world_timing() {
    let settings = mothr_core::Settings::default();
    eprintln!("roots: {:?}", settings.project_roots);
    let index = ProjectIndex::with_options(IndexOptions {
        store_path: None,
        seed_from_history: false,
    });
    let t = Instant::now();
    let walked = index.scan(&settings.project_roots, 4);
    eprintln!("walk only: {walked} projects in {:?}", t.elapsed());

    let t = Instant::now();
    let history = discover::from_history(&home_dir());
    eprintln!(
        "history seeding: {} projects in {:?}",
        history.len(),
        t.elapsed()
    );

    let index = ProjectIndex::with_options(IndexOptions {
        store_path: None,
        seed_from_history: true,
    });
    let t = Instant::now();
    let count = index.scan(&settings.project_roots, 4);
    eprintln!("full scan: {count} projects in {:?}", t.elapsed());
    for q in ["", "s", "shep", "dev core", "elbert"] {
        let t = Instant::now();
        let hits = index.search(q, 20);
        let el = t.elapsed();
        let top: Vec<String> = hits
            .iter()
            .take(3)
            .map(|h| tildify(&h.project.path))
            .collect();
        eprintln!("search {q:?}: {} hits in {el:?}; top {top:?}", hits.len());
    }
}

#[cfg(unix)]
#[test]
fn symlinked_root_is_scanned() {
    let tmp = fixture();
    let link = tempfile::tempdir().unwrap();
    let root = link.path().join("projects");
    std::os::unix::fs::symlink(tmp.path(), &root).unwrap();
    let index = mem_index();
    assert_eq!(index.scan(&[root], 4), 5);
}
