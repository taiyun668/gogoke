//! Internal launch-home resolution for the native H seam.
//!
//! Paths returned here stay inside the native crate. Resolution requires the
//! registered Codex home, current RootLock observations, and the ACTIVE H
//! owner binding to agree before a caller can use either path.

use super::registry::{resolve_registered_codex_home, RegistryError};
use super::temporary::{resolve_active_session_home, TemporaryHomeError};
use crate::process::AppContainerProfile;
use crate::root::{RootIdentity, RootLock};
use crate::store::same_open::VerifiedDatabaseConnection;
use std::path::PathBuf;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ResolvedDirectory {
    pub(crate) path: PathBuf,
    pub(crate) identity: RootIdentity,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct InstanceLaunchHomes {
    pub(crate) instance: ResolvedDirectory,
    pub(crate) session: ResolvedDirectory,
}

#[derive(Debug)]
pub(crate) enum LaunchHomeError {
    Registry(RegistryError),
    Temporary(TemporaryHomeError),
}

impl From<RegistryError> for LaunchHomeError {
    fn from(error: RegistryError) -> Self {
        Self::Registry(error)
    }
}

impl From<TemporaryHomeError> for LaunchHomeError {
    fn from(error: TemporaryHomeError) -> Self {
        Self::Temporary(error)
    }
}

pub(crate) fn resolve_codex_instance_home(
    connection: &VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    instance_id: &str,
) -> Result<ResolvedDirectory, LaunchHomeError> {
    let (path, identity) = resolve_registered_codex_home(connection, root, instance_id)?;
    Ok(ResolvedDirectory { path, identity })
}

/// Resolve both paths needed by an H Codex launch. The first resolution also
/// establishes that the requested instance is registered as Codex before the
/// session-home resolver is allowed to return a path.
pub(crate) fn resolve_codex_session_launch_homes(
    connection: &VerifiedDatabaseConnection<'_>,
    root: &RootLock,
    profile: &AppContainerProfile,
    instance_id: &str,
    home_id: &str,
    domain_id: &str,
    owner_id: &str,
    generation: &str,
) -> Result<InstanceLaunchHomes, LaunchHomeError> {
    let instance = resolve_codex_instance_home(connection, root, instance_id)?;
    let (path, identity) = resolve_active_session_home(
        connection,
        root,
        profile,
        instance_id,
        home_id,
        domain_id,
        owner_id,
        generation,
    )?;
    Ok(InstanceLaunchHomes {
        instance,
        session: ResolvedDirectory { path, identity },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::root::inspect_root;
    use crate::store::authority::initialize_process_custody_schema;
    use crate::store::digest::sha256_hex;
    use crate::store::instance::initialize_schema;
    use crate::store::instance::registry::{register_instance, ProgramObservation, Registration};
    use crate::store::instance::temporary::{
        create_temporary_home, CreateTemporaryHome, TemporaryKind,
    };
    use crate::store::same_open::{create_new, route_b_test_guard};
    use crate::store::session_transport::{
        bind_owner_in_transaction, initialize_admission_schema, OwnerBinding,
    };
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture(
        run: impl FnOnce(&mut VerifiedDatabaseConnection<'_>, &RootLock, &AppContainerProfile),
    ) {
        let _guard = route_b_test_guard();
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root_path = std::env::temp_dir().join(format!(
            "gogoke-v37-instance-resolver-{}-{nonce}",
            std::process::id()
        ));
        fs::create_dir(&root_path).unwrap();
        let root = RootLock::acquire(&root_path).unwrap();
        let database = root_path.join("state.sqlite");
        let mut connection = create_new(&root, &database).unwrap();
        connection.execute("PRAGMA foreign_keys=ON").unwrap();
        initialize_schema(&mut connection).unwrap();
        initialize_admission_schema(&mut connection).unwrap();
        initialize_process_custody_schema(&mut connection).unwrap();

        let program_path = root_path.join("test-program.bin");
        fs::write(&program_path, b"fixture program bytes").unwrap();
        let program = ProgramObservation::observe(&program_path, "0.149.0").unwrap();
        register_instance(
            &mut connection,
            &root,
            &Registration {
                request_id: "instanceReg",
                request_bytes: b"register instance",
                instance_id: "instanceA",
                driver_id: "codex",
                program: &program,
            },
        )
        .unwrap();
        connection.execute("BEGIN IMMEDIATE").unwrap();
        bind_owner_in_transaction(
            &mut connection,
            &OwnerBinding {
                binding_id: "bindingA",
                instance_id: "instanceA",
                domain_id: "projectA",
                kind: "SESSION",
                owner_id: "sessionA",
                generation: "1",
            },
        )
        .unwrap();
        connection.execute("COMMIT").unwrap();
        let profile = AppContainerProfile::derived_for_test("Gogoke37.FResolver").unwrap();
        run(&mut connection, &root, &profile);
        connection.close_checked().unwrap();
        drop(root);
        fs::remove_dir_all(root_path).unwrap();
    }

    fn create_session_home(
        connection: &mut VerifiedDatabaseConnection<'_>,
        root: &RootLock,
        profile: &AppContainerProfile,
    ) {
        create_temporary_home(
            connection,
            root,
            profile,
            &CreateTemporaryHome {
                request_id: "tempCreate",
                request_bytes: b"create session",
                home_id: "tempA",
                instance_id: "instanceA",
                domain_id: "projectA",
                kind: TemporaryKind::Session,
                owner_id: "sessionA",
                generation: "1",
            },
        )
        .unwrap();
    }

    #[test]
    fn resolves_registered_codex_and_active_session_paths_with_current_identities() {
        fixture(|connection, root, profile| {
            create_session_home(connection, root, profile);
            let resolved = resolve_codex_session_launch_homes(
                connection,
                root,
                profile,
                "instanceA",
                "tempA",
                "projectA",
                "sessionA",
                "1",
            )
            .unwrap();
            let expected_instance = root
                .canonical_root()
                .canonical_path
                .join("v37-instances")
                .join("instanceA");
            let expected_session = expected_instance
                .join("temporary-homes")
                .join(format!("temp-home-{}", sha256_hex(b"tempA")));
            assert_eq!(resolved.instance.path, expected_instance);
            assert_eq!(resolved.session.path, expected_session);
            assert_eq!(
                resolved.instance.identity,
                inspect_root(&resolved.instance.path).unwrap().identity
            );
            assert_eq!(
                resolved.session.identity,
                inspect_root(&resolved.session.path).unwrap().identity
            );
        });
    }

    #[test]
    fn rejects_owner_generation_lifecycle_and_identity_mismatch() {
        fixture(|connection, root, profile| {
            create_session_home(connection, root, profile);
            assert!(matches!(
                resolve_codex_session_launch_homes(
                    connection,
                    root,
                    profile,
                    "instanceA",
                    "tempA",
                    "projectA",
                    "otherOwner",
                    "1",
                ),
                Err(LaunchHomeError::Temporary(TemporaryHomeError::Unknown))
            ));
            assert!(matches!(
                resolve_codex_session_launch_homes(
                    connection,
                    root,
                    profile,
                    "instanceA",
                    "tempA",
                    "projectA",
                    "sessionA",
                    "2",
                ),
                Err(LaunchHomeError::Temporary(TemporaryHomeError::Unknown))
            ));
            connection
                .execute("UPDATE main.gogoke_v37_instance_homes SET state='UNKNOWN' WHERE home_id='tempA'")
                .unwrap();
            assert!(matches!(
                resolve_codex_session_launch_homes(
                    connection,
                    root,
                    profile,
                    "instanceA",
                    "tempA",
                    "projectA",
                    "sessionA",
                    "1",
                ),
                Err(LaunchHomeError::Temporary(TemporaryHomeError::Unknown))
            ));
            connection
                .execute("UPDATE main.gogoke_v37_instance_homes SET state='ACTIVE',directory_identity='changed' WHERE home_id='tempA'")
                .unwrap();
            assert!(matches!(
                resolve_codex_session_launch_homes(
                    connection,
                    root,
                    profile,
                    "instanceA",
                    "tempA",
                    "projectA",
                    "sessionA",
                    "1",
                ),
                Err(LaunchHomeError::Temporary(
                    TemporaryHomeError::IdentityChanged
                ))
            ));
            connection
                .execute("UPDATE main.gogoke_v37_instances SET home_identity='changed' WHERE instance_id='instanceA'")
                .unwrap();
            assert!(matches!(
                resolve_codex_instance_home(connection, root, "instanceA"),
                Err(LaunchHomeError::Registry(RegistryError::IdentityChanged))
            ));
        });
    }
}
