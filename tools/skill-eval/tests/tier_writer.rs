#[macro_export]
macro_rules! tier_writer_tests {
    () => {
        mod tier_test_support {
            pub(super) use std::fs;
            #[cfg(unix)]
            pub(super) use std::os::unix::fs::PermissionsExt;
            pub(super) use std::path::{Path, PathBuf};
            use std::sync::atomic::{AtomicU64, Ordering};

            pub(super) use $crate::model::{
                ArtifactChange, OwnEvalEvidence, PublicationGate, PublicationStatus, Tier,
                TierAssignment, TierDestination,
            };
            pub(super) use $crate::ports::ArtifactSource;
            pub(super) use $crate::source::FileArtifactSource;

            static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

            pub(super) struct TierFixture {
                pub(super) root: PathBuf,
            }

            impl TierFixture {
                pub(super) fn new(kind: &str) -> Self {
                let id = NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed);
                let root = std::env::temp_dir().join(format!(
                    "skill-eval-tier-writer-{kind}-{}-{id}",
                    std::process::id()
                ));
                fs::create_dir_all(root.join("evals")).unwrap();
                fs::write(
                    root.join("evals/cases.jsonl"),
                    "{\"id\":\"case-1\",\"input\":\"input\",\"expect\":\"expect\",\"source\":\"source\",\"holdout\":false,\"execution\":{\"drive\":{\"kind\":\"response\"},\"allowed_tools\":[],\"timeout_seconds\":1}}\n",
                )
                .unwrap();
                Self { root }
            }

                pub(super) fn write(&self, path: &str, content: &str) {
                let path = self.root.join(path);
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).unwrap();
                }
                fs::write(path, content).unwrap();
            }

                pub(super) fn load(&self) -> $crate::model::ArtifactDefinition {
                FileArtifactSource.load(&self.root).unwrap()
            }
        }

        impl Drop for TierFixture {
            fn drop(&mut self) {
                let _ = fs::remove_dir_all(&self.root);
            }
        }

            pub(super) fn tier_assignment(
                destination: TierDestination,
                tier: Tier,
            ) -> TierAssignment {
                TierAssignment { destination, tier }
            }

            pub(super) fn assert_no_transaction_files(root: &Path) {
                let names = fs::read_dir(root)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .filter(|name| {
                        name.contains(".tier-write-")
                            || name.contains(".tier-backup-")
                            || name.contains(".tier-restore-")
                    })
                    .collect::<Vec<_>>();
                assert!(names.is_empty(), "transaction files remain: {names:?}");
            }

            pub(super) fn assert_no_staged_transaction_files(root: &Path) {
                let names = fs::read_dir(root)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .filter(|name| name.contains(".tier-write-"))
                    .collect::<Vec<_>>();
                assert!(names.is_empty(), "staged transaction files remain: {names:?}");
            }

            pub(super) fn ready_gate(
            artifact: &$crate::model::ArtifactDefinition,
            assignments: Vec<TierAssignment>,
        ) -> PublicationGate {
            PublicationGate {
                change: ArtifactChange {
                    artifact: artifact.name.clone(),
                    kind: artifact.kind,
                    incumbent_revision: "incumbent".to_owned(),
                    candidate_revision: artifact.revision.clone(),
                    own_eval: OwnEvalEvidence {
                        artifact_revision: artifact.revision.clone(),
                        path: PathBuf::from("evals/result.json"),
                    },
                },
                status: PublicationStatus::Ready,
                assignments,
                    reason: None,
                }
            }
        }

        use tier_test_support::*;

        #[test]
        fn tc_39_type_owned_destinations() {
            let skill = TierFixture::new("skill");
            let skill_before = "---\nname: fixture-skill\ndescription: Exact bytes.\nmetadata:\n  owner: keep\n  minimum-tier: T4\n  target-tier: T3\n---\nbody: model stays in prose\n";
            skill.write("SKILL.md", skill_before);
            let artifact = skill.load();
            let assignments = vec![
                tier_assignment(TierDestination::SkillMinimum, Tier::T2),
                tier_assignment(TierDestination::SkillTarget, Tier::T2),
            ];
            FileTierWriter.write(&artifact, &assignments).unwrap();
            assert_eq!(
                fs::read_to_string(skill.root.join("SKILL.md")).unwrap(),
                skill_before.replace("minimum-tier: T4", "minimum-tier: T2").replace(
                    "target-tier: T3",
                    "target-tier: T2"
                )
            );

            let agent = TierFixture::new("agent");
            agent.write(
                "fixture-agent.md",
                "---\nname: fixture-agent\ndescription: Exact agent.\ntools: Read\nmodel: sonnet\n---\nbody\n",
            );
            let routing = "{\n  \"tiers\": {\"T1\": {\"model\": \"tiny\"}, \"T2\": {\"model\": \"sonnet\"}},\n  \"agents\": {\"other\": \"T1\", \"fixture-agent\": \"T1\"},\n  \"keep\": [1, 2, 3]\n}\n";
            agent.write("config/model-tiers.json", routing);
            let artifact = agent.load();
            FileTierWriter
                .write(
                    &artifact,
                    &[tier_assignment(TierDestination::Agent, Tier::T2)],
                )
                .unwrap();
            assert_eq!(
                fs::read_to_string(agent.root.join("config/model-tiers.json")).unwrap(),
                routing.replace("\"fixture-agent\": \"T1\"", "\"fixture-agent\": \"T2\"")
            );

            let workflow = TierFixture::new("workflow");
            let definition = "---\nname: fixture-workflow\ndescription: Exact workflow.\nmetadata:\n  owner: keep\n  minimum-tier: T4\n---\nbody\n";
            let executable = "export const meta = { keep: 'yes', phases: [\n  { title: 'Plan', model: 'sonnet', keep: 1 },\n  { title: 'Review', tier: 'T4', keep: 2 },\n] };\n";
            workflow.write("SKILL.md", definition);
            workflow.write("fixture.workflow.js", executable);
            let artifact = workflow.load();
            FileTierWriter
                .write(
                    &artifact,
                    &[
                        tier_assignment(TierDestination::WorkflowOrchestrator, Tier::T2),
                        tier_assignment(
                            TierDestination::WorkflowNode {
                                node: "Plan".to_owned(),
                            },
                            Tier::T2,
                        ),
                        tier_assignment(
                            TierDestination::WorkflowNode {
                                node: "Review".to_owned(),
                            },
                            Tier::T3,
                        ),
                    ],
                )
                .unwrap();
            assert_eq!(
                fs::read_to_string(workflow.root.join("SKILL.md")).unwrap(),
                definition.replace("minimum-tier: T4", "minimum-tier: T2")
            );
            assert_eq!(
                fs::read_to_string(workflow.root.join("fixture.workflow.js")).unwrap(),
                executable
                    .replace("model: 'sonnet'", "tier: 'T2'")
                    .replace("tier: 'T4'", "tier: 'T3'")
            );
        }

        #[test]
        fn forged_mixed_tier_ready_gate_is_rejected_before_writer_call() {
            struct CountingWriter {
                calls: u32,
            }

            impl TierWriter for CountingWriter {
                fn write(
                    &mut self,
                    _: &$crate::model::ArtifactDefinition,
                    _: &[TierAssignment],
                ) -> Result<(), $crate::model::SkillEvalError> {
                    self.calls += 1;
                    Ok(())
                }
            }

            let fixture = TierFixture::new("mixed-gate");
            fixture.write(
                "SKILL.md",
                "---\nname: fixture-skill\ndescription: Mixed gate.\nmetadata:\n  minimum-tier: T3\n  target-tier: T3\n---\nbody\n",
            );
            let artifact = fixture.load();
            let gate = ready_gate(
                &artifact,
                vec![
                    tier_assignment(TierDestination::SkillMinimum, Tier::T2),
                    tier_assignment(TierDestination::SkillTarget, Tier::T3),
                ],
            );
            let mut writer = CountingWriter { calls: 0 };

            let result = $crate::service::apply_tier_assignments(&gate, &artifact, &mut writer);

            assert!(matches!(
                result,
                Err($crate::model::SkillEvalError::InvalidArguments(message))
                    if message == "ready gate tier assignments must use one accepted tier"
            ));
            assert_eq!(writer.calls, 0);
        }

        #[test]
        fn valid_ready_gate_invokes_writer_once() {
            struct CountingWriter {
                calls: u32,
            }

            impl TierWriter for CountingWriter {
                fn write(
                    &mut self,
                    _: &$crate::model::ArtifactDefinition,
                    _: &[TierAssignment],
                ) -> Result<(), $crate::model::SkillEvalError> {
                    self.calls += 1;
                    Ok(())
                }
            }

            let fixture = TierFixture::new("valid-gate");
            fixture.write(
                "SKILL.md",
                "---\nname: fixture-skill\ndescription: Valid gate.\nmetadata:\n  minimum-tier: T3\n---\nbody\n",
            );
            let artifact = fixture.load();
            let gate = ready_gate(
                &artifact,
                vec![tier_assignment(TierDestination::SkillMinimum, Tier::T2)],
            );
            let mut writer = CountingWriter { calls: 0 };

            $crate::service::apply_tier_assignments(&gate, &artifact, &mut writer).unwrap();

            assert_eq!(writer.calls, 1);
        }

        #[test]
        fn rejects_non_ready_identity_assignment_and_revision_failures_before_write() {
            struct PanicWriter;
            impl TierWriter for PanicWriter {
                fn write(
                    &mut self,
                    _: &$crate::model::ArtifactDefinition,
                    _: &[TierAssignment],
                ) -> Result<(), $crate::model::SkillEvalError> {
                    panic!("writer must not be called")
                }
            }

            let fixture = TierFixture::new("gate");
            fixture.write(
                "SKILL.md",
                "---\nname: fixture-skill\ndescription: Gate.\nmetadata:\n  minimum-tier: T3\n---\nbody\n",
            );
            let artifact = fixture.load();
            let assignments = vec![tier_assignment(TierDestination::SkillMinimum, Tier::T2)];
            let mut gate = ready_gate(&artifact, assignments.clone());
            gate.status = PublicationStatus::Blocked;
            assert!($crate::service::apply_tier_assignments(
                &gate,
                &artifact,
                &mut PanicWriter
            )
            .is_err());

            let mut gate = ready_gate(&artifact, assignments.clone());
            gate.change.kind = ArtifactKind::Agent;
            assert!($crate::service::apply_tier_assignments(
                &gate,
                &artifact,
                &mut PanicWriter
            )
            .is_err());

            let mut gate = ready_gate(&artifact, Vec::new());
            assert!($crate::service::apply_tier_assignments(
                &gate,
                &artifact,
                &mut PanicWriter
            )
            .is_err());
            gate.assignments = vec![
                assignments[0].clone(),
                tier_assignment(TierDestination::SkillTarget, Tier::T2),
            ];
            assert!($crate::service::apply_tier_assignments(
                &gate,
                &artifact,
                &mut PanicWriter
            )
            .is_err());

            let stale = artifact.clone();
            fixture.write(
                "SKILL.md",
                "---\nname: fixture-skill\ndescription: Gate.\nmetadata:\n  minimum-tier: T3\n---\nconcurrent body change\n",
            );
            assert!(FileTierWriter.write(&stale, &assignments).is_err());
        }

        #[test]
        fn forged_destinations_and_staging_failure_preserve_originals() {
            let workflow = TierFixture::new("forged-destination");
            workflow.write(
                "SKILL.md",
                "---\nname: fixture-workflow\ndescription: Forged destination.\nmetadata:\n  minimum-tier: T3\n---\nbody\n",
            );
            workflow.write(
                "fixture.workflow.js",
                "export const meta = { phases: [{ title: 'Plan', model: 'sonnet' }] };\n",
            );
            let mut artifact = workflow.load();
            let definition_before = fs::read(workflow.root.join("SKILL.md")).unwrap();
            let executable_before = fs::read(workflow.root.join("fixture.workflow.js")).unwrap();
            artifact.required_destinations[1] = TierDestination::WorkflowNode {
                node: "Missing".to_owned(),
            };
            assert!(FileTierWriter
                .write(
                    &artifact,
                    &[
                        tier_assignment(TierDestination::WorkflowOrchestrator, Tier::T2),
                        tier_assignment(
                            TierDestination::WorkflowNode {
                                node: "Missing".to_owned(),
                            },
                            Tier::T2,
                        ),
                    ],
                )
                .is_err());
            assert_eq!(fs::read(workflow.root.join("SKILL.md")).unwrap(), definition_before);
            assert_eq!(
                fs::read(workflow.root.join("fixture.workflow.js")).unwrap(),
                executable_before
            );
            assert_no_transaction_files(&workflow.root);

            let skill = TierFixture::new("atomic");
            skill.write(
                "SKILL.md",
                "---\nname: fixture-skill\ndescription: Atomic.\nmetadata:\n  minimum-tier: T3\n---\nbody\n",
            );
            let artifact = skill.load();
            let original = fs::read(skill.root.join("SKILL.md")).unwrap();
            for suffix in 0..1000_u16 {
                fs::write(
                    skill.root.join(format!(".SKILL.md.tier-write-{suffix}")),
                    "occupied",
                )
                .unwrap();
            }
            assert!(FileTierWriter
                .write(
                    &artifact,
                    &[tier_assignment(TierDestination::SkillMinimum, Tier::T2)],
                )
                .is_err());
            assert_eq!(fs::read(skill.root.join("SKILL.md")).unwrap(), original);
            assert!(
                fs::read_dir(&skill.root)
                    .unwrap()
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .all(|name| !name.contains(".tier-backup-"))
            );
        }

        #[test]
        fn tc_41_backup_cleanup_failure_restores_all_originals_and_modes() {
            let fixture = TierFixture::new("cleanup-rollback");
            let first_path = fixture.root.join("SKILL.md");
            let second_path = fixture.root.join("fixture.workflow.js");
            let first_original = b"first original\n".to_vec();
            let second_original = b"second original\n".to_vec();
            fs::write(&first_path, &first_original).unwrap();
            fs::write(&second_path, &second_original).unwrap();
            #[cfg(unix)]
            {
                fs::set_permissions(&first_path, fs::Permissions::from_mode(0o640)).unwrap();
                fs::set_permissions(&second_path, fs::Permissions::from_mode(0o604)).unwrap();
            }
            let replacements = vec![
                Replacement {
                    path: first_path.clone(),
                    original: first_original.clone(),
                    output: b"first replacement\n".to_vec(),
                },
                Replacement {
                    path: second_path.clone(),
                    original: second_original.clone(),
                    output: b"second replacement\n".to_vec(),
                },
            ];
            let mut removals = 0_u8;

            let result = replace_atomically_with_operations(
                &replacements,
                |from, to| fs::rename(from, to),
                |path| {
                    removals += 1;
                    if removals == 2 {
                        return Err(std::io::Error::other(
                            "injected backup removal failure",
                        ));
                    }
                    fs::remove_file(path)
                },
            );

            assert!(matches!(
                result,
                Err($crate::model::SkillEvalError::Io { message, .. })
                    if message == "injected backup removal failure"
            ));
            assert_eq!(fs::read(&first_path).unwrap(), first_original);
            assert_eq!(fs::read(&second_path).unwrap(), second_original);
            #[cfg(unix)]
            {
                assert_eq!(
                    fs::metadata(&first_path).unwrap().permissions().mode() & 0o777,
                    0o640
                );
                assert_eq!(
                    fs::metadata(&second_path).unwrap().permissions().mode() & 0o777,
                    0o604
                );
            }
            assert_no_transaction_files(&fixture.root);
        }

        #[test]
        fn tc_41_workflow_rename_failures_restore_original_bytes_and_modes() {
            let fixture = TierFixture::new("rollback");
            let first_path = fixture.root.join("SKILL.md");
            let second_path = fixture.root.join("fixture.workflow.js");
            let first_original = b"first original\n".to_vec();
            let second_original = b"second original\n".to_vec();
            fs::write(&first_path, &first_original).unwrap();
            fs::write(&second_path, &second_original).unwrap();
            #[cfg(unix)]
            {
                fs::set_permissions(&first_path, fs::Permissions::from_mode(0o620)).unwrap();
                fs::set_permissions(&second_path, fs::Permissions::from_mode(0o644)).unwrap();
            }
            let replacements = vec![
                Replacement {
                    path: first_path.clone(),
                    original: first_original.clone(),
                    output: b"first replacement\n".to_vec(),
                },
                Replacement {
                    path: second_path.clone(),
                    original: second_original.clone(),
                    output: b"second replacement\n".to_vec(),
                },
            ];
            for failure_at in [3_u8, 4_u8] {
                let mut renames = 0_u8;
                let result = replace_atomically_with_rename(&replacements, |from, to| {
                    renames += 1;
                    if renames == failure_at {
                        return Err(std::io::Error::other("injected rename failure"));
                    }
                    fs::rename(from, to)
                });

                assert!(result.is_err());
                assert_eq!(fs::read(&first_path).unwrap(), first_original);
                assert_eq!(fs::read(&second_path).unwrap(), second_original);
                #[cfg(unix)]
                {
                    assert_eq!(
                        fs::metadata(&first_path).unwrap().permissions().mode() & 0o777,
                        0o620
                    );
                    assert_eq!(
                        fs::metadata(&second_path).unwrap().permissions().mode() & 0o777,
                        0o644
                    );
                }
                assert_no_transaction_files(&fixture.root);
            }
        }

        #[test]
        fn tc_41_rename_failure_reports_restoration_failure_and_cleans_staged_files() {
            let fixture = TierFixture::new("rollback-failure");
            let first_path = fixture.root.join("SKILL.md");
            let second_path = fixture.root.join("fixture.workflow.js");
            let first_original = b"first original\n".to_vec();
            let second_original = b"second original\n".to_vec();
            fs::write(&first_path, &first_original).unwrap();
            fs::write(&second_path, &second_original).unwrap();
            let replacements = vec![
                Replacement {
                    path: first_path,
                    original: first_original,
                    output: b"first replacement\n".to_vec(),
                },
                Replacement {
                    path: second_path,
                    original: second_original,
                    output: b"second replacement\n".to_vec(),
                },
            ];
            let mut renames = 0_u8;

            let result = replace_atomically_with_rename(&replacements, |from, to| {
                renames += 1;
                if renames == 4 {
                    return Err(std::io::Error::other("injected commit rename failure"));
                }
                if renames == 5 {
                    return Err(std::io::Error::other("injected restoration rename failure"));
                }
                fs::rename(from, to)
            });

            assert!(matches!(
                result,
                Err($crate::model::SkillEvalError::InvalidConfiguration(message))
                    if message.contains("tier write rollback failed")
                        && message.contains("injected restoration rename failure")
                        && message.contains("injected commit rename failure")
            ));
            assert_no_staged_transaction_files(&fixture.root);
        }
    };
}
