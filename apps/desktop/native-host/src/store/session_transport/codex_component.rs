//! The fixed CLI's own Code Mode executable is a runtime dependency, not a
//! caller-selected tool. Grant only its exact file to the existing LPAC SID.
use crate::process::AppContainerProfile;
use crate::root::RootIdentity;
use crate::store::instance::ProgramObservation;
use std::fs::{File, OpenOptions};
use std::os::windows::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

const VERSION: &str = "0.160.0";
const HELPER_NAME: &str = "codex-code-mode-host.exe";
// Official @openai/codex-win32-x64 0.160.0-win32-x64 npm archive member:
// package/vendor/x86_64-pc-windows-msvc/bin/codex-code-mode-host.exe.
const HELPER_DIGEST: &str =
    "sha256:1d448bfde19e7a280d600d8d0bcddf77afbe9feaec1e804905becc5f39bc9db6";
const FILE_SHARE_READ: u32 = 1;
const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;

pub(crate) struct BoundCodexComponent {
    path: PathBuf,
    identity: RootIdentity,
    // Retain a read-only, no-write/no-delete-sharing handle through actual
    // process stop. A later same-name replacement cannot inherit this grant.
    _file: File,
}

impl BoundCodexComponent {
    pub(crate) fn prepare(program: &Path, profile: &AppContainerProfile)
        -> Result<Self, String> {
        if program.file_name().and_then(|value| value.to_str()) != Some("codex.exe") {
            return Err("native Codex component: unexpected catalog entrypoint".into());
        }
        let directory = program.parent()
            .ok_or("native Codex component: catalog directory absent")?;
        Self::prepare_at(&directory.join(HELPER_NAME), HELPER_DIGEST, profile)
    }

    fn prepare_at(path: &Path, expected_digest: &str, profile: &AppContainerProfile)
        -> Result<Self, String> {
        let file = OpenOptions::new().read(true).share_mode(FILE_SHARE_READ)
            .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT).open(path)
            .map_err(|error| format!("native Codex component open: {error:?}"))?;
        let identity = AppContainerProfile::capture_program_identity(path)
            .map_err(|error| format!("native Codex component identity: {error:?}"))?;
        let observed = ProgramObservation::observe(path, VERSION)
            .map_err(|error| format!("native Codex component digest: {error:?}"))?;
        if !observed.matches_pin(expected_digest, VERSION) {
            return Err("native Codex component: fixed package member digest changed".into());
        }
        profile.grant_bound_program(path, &identity)
            .map_err(|error| format!("native Codex component grant: {error:?}"))?;
        let component = Self { path: path.to_owned(), identity, _file: file };
        component.verify(profile)?;
        Ok(component)
    }

    pub(crate) fn verify(&self, profile: &AppContainerProfile) -> Result<(), String> {
        profile.verify_bound_program_grant(&self.path, &self.identity)
            .map_err(|error| format!("native Codex component verify: {error:?}"))?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn fixed_codex_component_rejects_unpinned_bytes_before_grant_and_pins_exact_file() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let folder = std::env::temp_dir().join(format!("gogoke-code-mode-binding-{}-{nonce}",
            std::process::id()));
        std::fs::create_dir(&folder).unwrap();
        let path = folder.join(HELPER_NAME);
        let sibling = folder.join("other.exe");
        let bytes = b"controlled component bytes";
        std::fs::write(&path, bytes).unwrap();
        std::fs::write(&sibling, b"unrelated image").unwrap();
        let profile = AppContainerProfile::derived_for_test(&format!("Gogoke37.Component.{nonce}"))
            .unwrap();
        let identity = AppContainerProfile::capture_program_identity(&path).unwrap();
        assert!(BoundCodexComponent::prepare(&folder.join("codex.exe"), &profile).is_err(),
            "production entrypoint must reject fixture bytes against the official member pin");
        assert!(profile.verify_bound_program_grant(&path, &identity).is_err(),
            "digest rejection must not grant the file");
        let expected = crate::store::digest::content_hash(bytes);
        let held = BoundCodexComponent::prepare_at(&path, &expected, &profile).unwrap();
        held.verify(&profile).unwrap();
        let witness = profile.verify_bound_program_grant(&path, &identity).unwrap();
        assert_eq!(witness.rights, 0x0012_0089 | 0x0012_00a0);
        assert_eq!(witness.inheritance, 0);
        let sibling_identity = AppContainerProfile::capture_program_identity(&sibling).unwrap();
        assert!(profile.verify_bound_program_grant(&sibling, &sibling_identity).is_err());
        assert!(std::fs::write(&path, b"changed bytes").is_err(), "retained image cannot be written");
        assert!(std::fs::rename(&path, folder.join("replaced.exe")).is_err(),
            "retained image cannot be replaced by name");
        drop(held);
        std::fs::rename(&path, folder.join("old.exe")).unwrap();
        std::fs::write(&path, bytes).unwrap();
        assert!(profile.verify_bound_program_grant(&path, &identity).is_err(),
            "same bytes at a different physical file are not the old bound object");
        let alias = folder.join("alias.exe");
        std::fs::hard_link(&path, &alias).unwrap();
        assert!(BoundCodexComponent::prepare_at(&path, &expected, &profile).is_err(),
            "multi-link image must be rejected");
        std::fs::remove_file(&alias).unwrap();
        std::fs::remove_file(&path).unwrap();
        assert!(BoundCodexComponent::prepare_at(&path, &expected, &profile).is_err(),
            "missing image must be refused, never replaced with a directory grant");
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
