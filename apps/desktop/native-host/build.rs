use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

const SQLITE_C: &str = "vendor/sqlite-3.53.2/sqlite3.c";
const SQLITE_H: &str = "vendor/sqlite-3.53.2/sqlite3.h";
const SQLITE_PATCH: &str = "vendor/sqlite-3.53.2/patches/route_b_same_open.patch";
const EXPECTED_C: &str = "0a409f1633283fa31a9126b11fbfd64a1991c5d30defad07e5745d4667f5e23d";
const EXPECTED_H: &str = "9e69a1353a4288450b0d5239ede11fc7f1f4c8e5eb07491fc8317eacb5b7de7e";
const EXPECTED_PATCH: &str = "926327cdb6b60859f1d0e3504946c6dd8f6ed50f256b0c3786b2ea855ea1c1fd";
const EXPECTED_GENERATED: &str = "f664bfbb7f3112b1ea86d250b80f9a36c60dcf800566d33066ada06a262baa76";

fn sha256(input: &[u8]) -> String {
    const INITIAL: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let bit_length = (input.len() as u64) * 8;
    let mut padded = input.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_length.to_be_bytes());
    let mut hash = INITIAL;
    for block in padded.chunks_exact(64) {
        let mut words = [0u32; 64];
        for (index, word) in words[..16].iter_mut().enumerate() {
            *word = u32::from_be_bytes(block[index * 4..index * 4 + 4].try_into().unwrap());
        }
        for index in 16..64 {
            let s0 = words[index - 15].rotate_right(7)
                ^ words[index - 15].rotate_right(18)
                ^ (words[index - 15] >> 3);
            let s1 = words[index - 2].rotate_right(17)
                ^ words[index - 2].rotate_right(19)
                ^ (words[index - 2] >> 10);
            words[index] = words[index - 16]
                .wrapping_add(s0)
                .wrapping_add(words[index - 7])
                .wrapping_add(s1);
        }
        let [mut a, mut b, mut c, mut d, mut e, mut f, mut g, mut h] = hash;
        for index in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let t1 = h
                .wrapping_add(s1)
                .wrapping_add((e & f) ^ (!e & g))
                .wrapping_add(K[index])
                .wrapping_add(words[index]);
            let t2 = (a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22))
                .wrapping_add((a & b) ^ (a & c) ^ (b & c));
            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(t1);
            d = c;
            c = b;
            b = a;
            a = t1.wrapping_add(t2);
        }
        for (slot, value) in hash.iter_mut().zip([a, b, c, d, e, f, g, h]) {
            *slot = slot.wrapping_add(value);
        }
    }
    hash.iter()
        .fold(String::with_capacity(64), |mut output, word| {
            write!(output, "{word:08x}").expect("writing to String cannot fail");
            output
        })
}

fn verified_source(path: &str, expected: &str) -> String {
    let actual =
        sha256(&fs::read(path).unwrap_or_else(|error| panic!("cannot read {path}: {error}")));
    assert_eq!(
        actual, expected,
        "fixed SQLite source digest mismatch: {path}"
    );
    actual
}

fn patch_fixed_source(source: &str, patch: &str) -> String {
    let lines: Vec<&str> = patch.lines().collect();
    assert_eq!(lines.first().copied(), Some("GOGOKE_ROUTE_B_PATCH_V1"));
    let mut blocks = Vec::new();
    let mut cursor = 1usize;
    while cursor < lines.len() {
        if lines[cursor].is_empty() {
            cursor += 1;
            continue;
        }
        assert!(lines[cursor].starts_with("BEGIN "), "invalid patch block");
        let operation = lines[cursor][6..].to_owned();
        cursor += 1;
        assert_eq!(lines[cursor], "ANCHOR");
        cursor += 1;
        let mut old = None;
        let anchor;
        if operation == "REPLACE" {
            assert_eq!(lines[cursor], "OLD");
            anchor = "OLD".to_owned();
            cursor += 1;
            let start = cursor;
            while lines[cursor] != "CODE" {
                cursor += 1;
                assert!(cursor < lines.len(), "unterminated patch old section");
            }
            old = Some(lines[start..cursor].join("\n") + "\n");
            cursor += 1;
        } else {
            let anchor_start = cursor;
            while lines[cursor] != "CODE" {
                cursor += 1;
                assert!(cursor < lines.len(), "unterminated patch anchor section");
            }
            let anchor_lines = &lines[anchor_start..cursor];
            anchor = if anchor_lines.len() == 1 {
                anchor_lines[0].to_owned()
            } else {
                anchor_lines.join("\n") + "\n"
            };
            cursor += 1;
        }
        let start = cursor;
        while lines[cursor] != "END" {
            cursor += 1;
            assert!(cursor < lines.len(), "unterminated patch code section");
        }
        let code = if start == cursor {
            String::new()
        } else {
            lines[start..cursor].join("\n") + "\n"
        };
        cursor += 1;
        blocks.push((operation, anchor, old, code));
    }

    let mut generated = source.to_owned();
    for (operation, anchor, old, code) in blocks {
        match operation.as_str() {
            "INSERT_BEFORE" => {
                assert_eq!(
                    generated.match_indices(&anchor).count(),
                    1,
                    "anchor count for {operation}: {anchor:?}"
                );
                generated = generated.replacen(&anchor, &(code + &anchor), 1);
            }
            "INSERT_AFTER" => {
                assert_eq!(
                    generated.match_indices(&anchor).count(),
                    1,
                    "anchor count for {operation}: {anchor:?}"
                );
                let separator = if anchor.ends_with('\n') { "" } else { "\n" };
                generated = generated.replacen(&anchor, &(anchor.clone() + separator + &code), 1);
            }
            "REPLACE" => {
                let old = old.expect("replace old section");
                assert_eq!(
                    generated.match_indices(&old).count(),
                    1,
                    "replacement count for {:?} ({} bytes)",
                    &old[..old.len().min(160)],
                    old.len()
                );
                generated = generated.replacen(&old, &code, 1);
            }
            other => panic!("unknown Route-B patch operation: {other}"),
        }
    }
    generated
}

fn main() {
    println!("cargo:rerun-if-changed={SQLITE_C}");
    println!("cargo:rerun-if-changed={SQLITE_H}");
    println!("cargo:rerun-if-changed={SQLITE_PATCH}");
    let source = fs::read(SQLITE_C).expect("read fixed SQLite amalgamation");
    let patch = fs::read(SQLITE_PATCH).expect("read Route-B same-open patch");
    let upstream_c = sha256(&source);
    let patch_sha256 = sha256(&patch);
    assert_eq!(
        patch_sha256, EXPECTED_PATCH,
        "fixed Route-B patch digest mismatch"
    );
    assert_eq!(
        upstream_c, EXPECTED_C,
        "fixed SQLite source digest mismatch"
    );
    let generated = patch_fixed_source(
        std::str::from_utf8(&source).expect("fixed SQLite amalgamation is UTF-8"),
        std::str::from_utf8(&patch).expect("Route-B patch is UTF-8"),
    );
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    let generated_path = out_dir.join("sqlite3.gogoke.route_b.c");
    fs::write(&generated_path, generated.as_bytes()).expect("write patched SQLite amalgamation");
    let generated_sha256 = sha256(generated.as_bytes());
    assert_eq!(
        generated_sha256, EXPECTED_GENERATED,
        "generated SQLite source digest mismatch"
    );
    assert_route_b_test_hooks_are_ifdef_gated(&generated);
    println!("cargo:rustc-env=GOGOKE_SQLITE3_C_SHA256={}", upstream_c);
    println!(
        "cargo:rustc-env=GOGOKE_SQLITE3_H_SHA256={}",
        verified_source(SQLITE_H, EXPECTED_H)
    );
    println!("cargo:rustc-env=GOGOKE_SQLITE3_PATCH_SHA256={patch_sha256}");
    println!("cargo:rustc-env=GOGOKE_SQLITE3_GENERATED_C_SHA256={generated_sha256}");
    let mut cc = cc::Build::new();
    cc.file(&generated_path)
        .warnings(false)
        .define("SQLITE_THREADSAFE", "1")
        .define("SQLITE_DQS", "0")
        .define("SQLITE_DEFAULT_FOREIGN_KEYS", "1")
        .define("SQLITE_DEFAULT_WAL_SYNCHRONOUS", "2")
        .define("SQLITE_OMIT_LOAD_EXTENSION", None)
        .define("SQLITE_TRUSTED_SCHEMA", "0");
    // build.rs is never itself cfg(test), and CARGO_CFG_TEST is not set for
    // this cc unit either. Release is the publishable image: test hooks stay
    // compiled out there. Debug/test builds define the macro so the same
    // hooks remain available to cargo test.
    let testing_image = std::env::var("PROFILE").unwrap_or_default() != "release";
    if testing_image {
        cc.define("GOGOKE_ROUTE_B_TESTING", None);
    }
    println!(
        "cargo:rustc-env=GOGOKE_ROUTE_B_TESTING_COMPILED={}",
        if testing_image { "1" } else { "0" }
    );
    cc.compile("gogoke_sqlite3");
    if !testing_image {
        assert_release_object_omits_test_hooks(&out_dir);
    }
}

fn assert_release_object_omits_test_hooks(out_dir: &Path) {
    let lib = out_dir.join("gogoke_sqlite3.lib");
    let bytes = fs::read(&lib).unwrap_or_else(|error| {
        panic!(
            "failed to read release SQLite image {}: {error}",
            lib.display()
        )
    });
    assert!(
        !bytes.is_empty(),
        "release SQLite image is empty: {}",
        lib.display()
    );
    let as_ascii = String::from_utf8_lossy(&bytes);
    assert!(
        as_ascii.contains("sqlite3_gogoke_bind_main_handle"),
        "release SQLite image is missing the required Route-B binding symbol"
    );
    for hook in [
        "sqlite3_gogoke_test_enable_full_pathname_reentry",
        "sqlite3_gogoke_test_last_reentry_result",
        "sqlite3_gogoke_test_force_next_close_failure",
        "gogokeRouteBForceNextCloseFailure",
    ] {
        assert!(
            !as_ascii.contains(hook),
            "release SQLite image still contains test hook {hook}"
        );
    }
}

fn assert_route_b_test_hooks_are_ifdef_gated(generated: &str) {
    const HOOKS: [&str; 4] = [
        "sqlite3_gogoke_test_enable_full_pathname_reentry",
        "sqlite3_gogoke_test_last_reentry_result",
        "sqlite3_gogoke_test_force_next_close_failure",
        "gogokeRouteBForceNextCloseFailure",
    ];
    const IFDEF: &str = "#ifdef GOGOKE_ROUTE_B_TESTING";
    const ENDIF: &str = "#endif";
    for hook in HOOKS {
        let mut found = false;
        let mut search_from = 0usize;
        while let Some(rel) = generated[search_from..].find(hook) {
            found = true;
            let abs = search_from + rel;
            let before = &generated[..abs];
            let ifdef = before.rfind(IFDEF);
            let endif = before.rfind(ENDIF);
            assert!(
                ifdef.is_some() && (endif.is_none() || ifdef.unwrap() > endif.unwrap()),
                "{hook} must appear only inside {IFDEF}"
            );
            search_from = abs + hook.len();
        }
        assert!(found, "{hook} missing from generated SQLite image");
    }
}
