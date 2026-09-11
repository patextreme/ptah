//! Offline test fixtures (feature `test-fixtures`): git repositories
//! built directly through gix — no `git` binary, no network — for
//! package sources and registry indices. Integration tests and the
//! adapter's own unit tests share these builders so fixture shape and
//! pesde's expectations can never drift apart (format generation uses
//! pesde's own serialization where one exists).

use std::collections::BTreeMap;
use std::path::Path;

use gix::objs;
use gix::refs::transaction as refs_tx;

/// A built git repository.
#[derive(Debug, Clone)]
pub struct GitFixture {
    /// The repository directory (usable as a git URL for clones).
    pub dir: std::path::PathBuf,
    /// The tip commit id of `main`.
    pub head: gix::ObjectId,
}

/// One file in a fixture repository tree.
pub struct FileSpec<'a> {
    pub path: &'a str,
    pub contents: &'a str,
}

/// A tree node: a blob id or a subtree.
enum Node {
    Blob(gix::ObjectId),
    Tree(BTreeMap<String, Node>),
}

/// Write a git repository at `dir` with one commit on `main`
/// containing `files`. The worktree is not checked out — pesde reads
/// committed trees, not working directories.
pub fn git_repo(dir: &Path, files: &[FileSpec<'_>]) -> GitFixture {
    git_repo_with(dir, files, "refs/heads/main")
}

/// [`git_repo`] with an explicit branch (used to build multi-branch
/// fixtures such as an index that later serves a newer version).
pub fn git_repo_with(dir: &Path, files: &[FileSpec<'_>], branch: &str) -> GitFixture {
    let existing = dir.join(".git").exists();
    let repo = if existing {
        gix::open(dir).expect("git open")
    } else {
        gix::init(dir).expect("git init")
    };

    // Build the tree bottom-up.
    let mut root: BTreeMap<String, Node> = BTreeMap::new();
    for file in files {
        let blob = repo
            .write_object(objs::Blob {
                data: file.contents.as_bytes().into(),
            })
            .expect("write blob");
        let mut node = &mut root;
        let segments: Vec<&str> = file.path.split('/').collect();
        for (i, segment) in segments.iter().enumerate() {
            if i + 1 == segments.len() {
                node.insert(
                    (*segment).to_string(),
                    Node::Blob(blob.detach()),
                );
            } else {
                node = match node
                    .entry((*segment).to_string())
                    .or_insert_with(|| Node::Tree(BTreeMap::new()))
                {
                    Node::Tree(map) => map,
                    Node::Blob(_) => panic!("fixture path conflict at {segment}"),
                };
            }
        }
    }
    let tree_id = write_tree(&repo, &root);

    let signature = gix::actor::Signature {
        name: "ptah fixture".into(),
        email: "fixture@ptah.invalid".into(),
        time: gix::date::Time::new(0, 0),
    };
    // The branch's current tip (if any) becomes the parent, so an
    // evolving fixture keeps a history.
    let parent = repo
        .find_reference(branch)
        .ok()
        .and_then(|r| r.try_id().map(|id| id.detach()));
    let commit = repo
        .write_object(objs::Commit {
            tree: tree_id,
            parents: parent.clone().into_iter().collect(),
            author: signature.clone(),
            committer: signature,
            encoding: None,
            message: "fixture".into(),
            extra_headers: Default::default(),
        })
        .expect("write commit")
        .detach();

    // Ref edits carry the fixture signature as the committer: the
    // build sandbox (and any machine without git user configuration)
    // has no identity for reflog entries to inherit.
    let committer = gix::actor::Signature {
        name: "ptah fixture".into(),
        email: "fixture@ptah.invalid".into(),
        time: gix::date::Time::new(0, 0),
    };
    let expected = match &parent {
        Some(tip) => refs_tx::PreviousValue::MustExistAndMatch(gix::refs::Target::Object(*tip)),
        None => refs_tx::PreviousValue::Any,
    };
    let mut edits = vec![refs_tx::RefEdit {
        change: refs_tx::Change::Update {
            log: Default::default(),
            new: gix::refs::Target::Object(commit),
            expected,
        },
        name: gix::refs::FullName::try_from(branch).expect("valid branch name"),
        deref: false,
    }];
    if !existing {
        // HEAD points at the fixture branch (init's default branch
        // name varies with configuration; pin it so clones resolve
        // HEAD).
        edits.push(refs_tx::RefEdit {
            change: refs_tx::Change::Update {
                log: Default::default(),
                new: gix::refs::Target::Symbolic(
                    gix::refs::FullName::try_from(branch).expect("valid branch name"),
                ),
                expected: refs_tx::PreviousValue::Any,
            },
            name: gix::refs::FullName::try_from("HEAD").expect("HEAD is a valid ref name"),
            deref: false,
        });
    }
    let mut time_buf = gix::date::parse::TimeBuf::default();
    repo.edit_references_as(edits, Some(committer.to_ref(&mut time_buf)))
        .expect("write fixture refs");

    GitFixture {
        dir: dir.to_path_buf(),
        head: commit,
    }
}

/// Recursively write tree objects for a node map, returning the root
/// tree id.
fn write_tree(repo: &gix::Repository, map: &BTreeMap<String, Node>) -> gix::ObjectId {
    let mut entries: Vec<objs::tree::Entry> = Vec::new();
    for (name, node) in map {
        let (mode, id) = match node {
            Node::Blob(id) => (objs::tree::EntryKind::Blob.into(), *id),
            Node::Tree(sub) => (objs::tree::EntryKind::Tree.into(), write_tree(repo, sub)),
        };
        entries.push(objs::tree::Entry {
            mode,
            filename: name.as_str().into(),
            oid: id,
        });
    }
    repo.write_object(objs::Tree { entries })
        .expect("write tree")
        .detach()
}

/// One published version in a fixture registry index.
pub struct IndexEntrySpec<'a> {
    /// Full package name, `scope/name`.
    pub name: &'a str,
    /// The published version, e.g. `0.1.0`.
    pub version: &'a str,
    /// The `lib` export path of the package (e.g. `init.luau`).
    pub lib: Option<&'a str>,
}

/// Build a fixture registry index at `dir`: a git repository whose
/// root carries `config.toml` (pointing at `api`) and one index file
/// per package (`<scope>/<name>`), serialized through pesde's own
/// `IndexFile` type so the format cannot drift.
pub fn registry_index(dir: &Path, api: &str, packages: &[IndexEntrySpec<'_>]) -> GitFixture {
    // Owned contents first: FileSpec borrows them.
    let mut owned: Vec<(String, String)> = vec![(
        "config.toml".to_string(),
        format!("api = {api:?}\n"),
    )];
    for package in packages {
        let (scope, name) = package.name.split_once('/').expect("scope/name");
        let entry = pesde::source::pesde::IndexFileEntry {
            target: pesde::manifest::target::Target::Luau {
                lib: package
                    .lib
                    .map(|lib| relative_path::RelativePathBuf::from(lib)),
                bin: None,
                scripts: Default::default(),
            },
            published_at: "2020-01-01T00:00:00Z"
                .parse()
                .expect("static timestamp parses"),
            engines: Default::default(),
            description: None,
            license: None,
            authors: Vec::new(),
            repository: None,
            docs: Default::default(),
            yanked: false,
            dependencies: Default::default(),
        };
        let mut entries = BTreeMap::new();
        entries.insert(
            format!("{} luau", package.version)
                .parse::<pesde::source::ids::VersionId>()
                .unwrap_or_else(|_| panic!("{} parses as a VersionId", package.version)),
            entry,
        );
        let index_file = pesde::source::pesde::IndexFile {
            meta: Default::default(),
            entries,
        };
        owned.push((
            format!("{scope}/{name}"),
            toml::to_string(&index_file).expect("serialize index file"),
        ));
    }
    let files: Vec<FileSpec<'_>> = owned
        .iter()
        .map(|(path, contents)| FileSpec { path, contents })
        .collect();
    git_repo(dir, &files)
}
