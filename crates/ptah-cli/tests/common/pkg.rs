//! The offline fixture registry: a generated local git index (via the
//! `ptah-pesde` fixture builders) plus an in-process loopback HTTP
//! server serving fixture package tarballs — the registry half of the
//! mock-agent pattern. Nothing here reaches the network: the index is
//! a local path (cloned by pesde's gix transport), and the archive
//! endpoint is a 127.0.0.1 listener.
//!
//! Serving shapes follow pesde's index/archive contracts: the index
//! repo's `config.toml` names this server's API base, per-package
//! index files are generated through pesde's own serialization, and
//! archives are `GET {api}/v1/packages/{scope/name}/{version}/{target}/archive`
//! returning a gzipped tarball of the package tree.
#![allow(dead_code)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};

/// One fixture package version served by the registry.
pub struct FixturePackage {
    /// Full package name, `scope/name`.
    pub name: &'static str,
    pub version: &'static str,
    /// The package's lib export path (`pesde.toml` `[target] lib`).
    pub lib: &'static str,
    /// The package tree served in the archive: relative path,
    /// contents.
    pub files: Vec<(&'static str, String)>,
}

impl FixturePackage {
    /// A `luau`-target package whose lib exports a typed table with
    /// one function.
    pub fn hello(version: &'static str, greeting: &'static str) -> Self {
        Self {
            name: "ptah_fixture/hello",
            version,
            lib: "init.luau",
            files: vec![
                (
                    "pesde.toml",
                    format!(
                        "name = \"ptah_fixture/hello\"\nversion = \"{version}\"\n\n[target]\nenvironment = \"luau\"\nlib = \"init.luau\"\n"
                    ),
                ),
                (
                    "init.luau",
                    format!(
                        "--!strict\nlocal M = {{}}\nfunction M.greet(name: string): string\n\treturn \"{greeting} \" .. name\nend\nreturn M\n"
                    ),
                ),
            ],
        }
    }
}

/// A running fixture registry: the git index directory (point the
/// manifest's `[indices]` — or `PTAH_DEFAULT_INDEX` — at it) and the
/// loopback archive server. Keep the value alive for the server's
/// thread to keep serving.
pub struct FixtureRegistry {
    /// The git index repository directory.
    pub index_dir: PathBuf,
    /// The loopback API base URL (`http://127.0.0.1:<port>`).
    pub api_url: String,
    server: std::sync::Arc<ServerHandle>,
}

struct ServerHandle {
    /// Packages by `name@version`; archives are built once on demand.
    archives: std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
    /// Names of packages the server must reject (registry-failure
    /// scenarios): responds 500.
    reject: std::sync::Mutex<std::collections::BTreeSet<String>>,
    listener_kill: std::sync::atomic::AtomicBool,
}

impl FixtureRegistry {
    /// Build and start a registry serving `packages` (the git index is
    /// generated alongside; rebuild with [`Self::rebuild_index`] when
    /// the served set changes).
    pub fn start(name: &str, packages: &[FixturePackage]) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        let api_url = format!("http://127.0.0.1:{port}");

        let handle = std::sync::Arc::new(ServerHandle {
            archives: std::sync::Mutex::new(BTreeMap::new()),
            reject: std::sync::Mutex::new(std::collections::BTreeSet::new()),
            listener_kill: std::sync::atomic::AtomicBool::new(false),
        });

        {
            let handle = handle.clone();
            std::thread::spawn(move || serve(listener, handle));
        }

        let index_dir = std::env::temp_dir().join(format!(
            "ptah-pkg-index-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&index_dir);
        let registry = Self {
            index_dir,
            api_url,
            server: handle,
        };
        registry.register(packages);
        registry
    }

    /// Serve (or re-serve) `packages`: builds their archives and
    /// regenerates the git index. Calling again with a larger set is
    /// the "registry later serves a newer version" fixture.
    pub fn register(&self, packages: &[FixturePackage]) {
        {
            let mut archives = self.server.archives.lock().unwrap();
            for package in packages {
                archives.insert(
                    format!("{}@{}", package.name, package.version),
                    tar_gz(&package.files),
                );
            }
        }
        let specs: Vec<ptah_pesde::fixtures::IndexEntrySpec<'_>> = packages
            .iter()
            .map(|p| ptah_pesde::fixtures::IndexEntrySpec {
                name: p.name,
                version: p.version,
                lib: Some(p.lib),
            })
            .collect();
        ptah_pesde::fixtures::registry_index(&self.index_dir, &self.api_url, &specs);
    }

    /// Make the server reject every request for `name` with a 500
    /// (registry-failure scenarios).
    pub fn reject(&self, name: &str) {
        self.server.reject.lock().unwrap().insert(name.to_string());
    }

    /// The index directory as a git URL for manifests/env.
    pub fn index_url(&self) -> String {
        format!("file://{}", self.index_dir.display())
    }
}

impl Drop for FixtureRegistry {
    fn drop(&mut self) {
        self.server
            .listener_kill
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

/// Accept loop: one thread per registry, sequential connections (the
/// test suite's concurrency is bounded by pesde's download semaphore,
/// and a single-threaded accept loop keeps the fixture deterministic).
fn serve(listener: TcpListener, handle: std::sync::Arc<ServerHandle>) {
    listener
        .set_nonblocking(true)
        .expect("listener non-blocking");
    loop {
        if handle.listener_kill.load(std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        match listener.accept() {
            Ok((stream, _)) => {
                handle_connection(stream, &handle);
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            Err(_) => return,
        }
    }
}

/// Minimal HTTP/1.1: read the request head, route one GET, answer
/// with Content-Length + Connection: close.
fn handle_connection(mut stream: TcpStream, handle: &ServerHandle) {
    let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
    let mut buffer = [0u8; 8192];
    let mut head = Vec::new();
    loop {
        match stream.read_return(&mut buffer) {
            Ok(0) => break,
            Ok(n) => {
                head.extend_from_slice(&buffer[..n]);
                if head.windows(4).any(|w| w == b"\r\n\r\n") {
                    break;
                }
            }
            Err(_) => return,
        }
    }
    let head = String::from_utf8_lossy(&head);
    let Some(path) = head.split_whitespace().nth(1) else {
        return;
    };

    // Route: /v1/packages/{scope%2Fname}/{version}/{target}/archive
    // (pesde URL-encodes the package name; split the raw path so the
    // encoded slash stays inside one segment, then decode it).
    let segments: Vec<&str> = path.trim_matches('/').split('/').collect();
    if segments.len() == 6
        && segments[0] == "v1"
        && segments[1] == "packages"
        && segments[5] == "archive"
    {
        let name = percent_decode(segments[2]);
        let version = segments[3].to_string();
        if handle.reject.lock().unwrap().contains(&name) {
            respond(&mut stream, 500, "text/plain", b"internal registry error");
            return;
        }
        let key = format!("{name}@{version}");
        match handle.archives.lock().unwrap().get(&key) {
            Some(body) => respond(&mut stream, 200, "application/octet-stream", body),
            None => respond(&mut stream, 404, "text/plain", b"not found"),
        }
    } else {
        respond(&mut stream, 404, "text/plain", b"not found");
    }
}

trait ReadReturn {
    fn read_return(&mut self, buf: &mut [u8]) -> std::io::Result<usize>;
}

impl ReadReturn for TcpStream {
    fn read_return(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        std::io::Read::read(self, buf)
    }
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &[u8]) {
    let reason = match status {
        200 => "OK",
        404 => "Not Found",
        _ => "Internal Server Error",
    };
    let head = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body);
    let _ = stream.flush();
}

/// Decode `%XX` escapes (pesde URL-encodes the `scope/name` package
/// name into the archive path).
fn percent_decode(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(byte) = u8::from_str_radix(hex, 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Build a gzipped ustar tarball of `files`.
fn tar_gz(files: &[(&str, String)]) -> Vec<u8> {
    let mut tar: Vec<u8> = Vec::new();
    let mut names: Vec<&( &str, String)> = files.iter().collect();
    names.sort_by_key(|(path, _)| *path);
    for (path, contents) in names {
        tar_header(&mut tar, path, contents.len() as u64);
        tar.extend_from_slice(contents.as_bytes());
        let pad = (512 - contents.len() % 512) % 512;
        tar.extend(std::iter::repeat_n(0u8, pad));
    }
    // Two zero blocks + pad to the 10240-byte record size readers
    // expect from streaming tars.
    tar.extend(std::iter::repeat_n(0u8, 1024));
    while tar.len() % 10240 != 0 {
        tar.push(0);
    }

    let mut encoder =
        flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&tar).expect("write tar");
    encoder.finish().expect("finish gzip")
}

/// One 512-byte ustar header.
fn tar_header(out: &mut Vec<u8>, path: &str, size: u64) {
    let mut header = [0u8; 512];
    let name = path.as_bytes();
    assert!(name.len() <= 100, "fixture paths fit ustar name field");
    header[..name.len()].copy_from_slice(name);
    header[100..108].copy_from_slice(b"0000644\0"); // mode
    header[108..116].copy_from_slice(b"0000000\0"); // uid
    header[116..124].copy_from_slice(b"0000000\0"); // gid
    let size_field = format!("{size:011o}");
    header[124..124 + 11].copy_from_slice(size_field.as_bytes());
    header[136..136 + 11].copy_from_slice(b"00000000000"); // mtime
    header[148..156].fill(b' '); // checksum placeholder
    header[156] = b'0'; // regular file
    header[257..262].copy_from_slice(b"ustar"); // magic
    header[263..265].copy_from_slice(b"00"); // version
    let checksum: u32 = header.iter().map(|&b| b as u32).sum();
    let checksum_field = format!("{checksum:06o}\0 ");
    header[148..156].copy_from_slice(checksum_field.as_bytes());
    out.extend_from_slice(&header);
}

/// Path helper for tests that want the fixture tarball on disk.
#[allow(dead_code)]
pub fn write_tar_gz(path: &Path, files: &[(&str, String)]) {
    std::fs::write(path, tar_gz(files)).unwrap();
}
