//! Opt-in installed-SDK gate. Creates only private synthetic native sessions;
//! imports SessionManager, never an Agent/model/authentication API.
// The opt-in failure injector uses an executable POSIX Node wrapper.
#![cfg(unix)]
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{io::Write, path::PathBuf};
use velune_agent_runtime::{
    Config,
    execution::ExecutionTarget,
    history::{self, PiForkBoundary},
};

struct SyntheticHome(PathBuf);

impl Drop for SyntheticHome {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            eprintln!("synthetic fork HOME cleanup failed: {error}");
        }
    }
}

#[test]
#[ignore = "requires explicitly authorized installed Pi SDK and Node; run with --ignored"]
fn pi_edit_fork_preserves_later_prefix_edits_deletions_images_tool_pairs_and_custom() {
    let binary = std::env::var("VELUNE_TEST_PI_BINARY")
        .expect("enabled SDK test requires VELUNE_TEST_PI_BINARY");
    let node = std::env::var("VELUNE_TEST_NODE_BINARY")
        .expect("enabled SDK test requires VELUNE_TEST_NODE_BINARY");
    let mut nonce = [0; 8];
    getrandom::fill(&mut nonce).unwrap();
    let root = std::env::temp_dir().join(format!(
        "velune-pi-edit-fork-{:x}",
        u64::from_le_bytes(nonce)
    ));
    std::fs::create_dir(&root).unwrap();
    let _cleanup = SyntheticHome(root.clone());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    std::fs::create_dir(root.join("sessions")).unwrap();
    let resources = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("resources");
    let config: Config =
        serde_json::from_value(json!({"execution":{"kind":"local"},"os_home":root,
        "binary":binary,"node_binary":node,"sdk_helper":resources.join("pi_sessions.mjs"),
        "agent_dir":root,"session_dir":root.join("sessions")}))
        .unwrap();
    for mode in [
        "user-leaf",
        "carry-edits",
        "compaction-carry-edits",
        "empty",
        "stale",
        "sync-file-fail",
        "sync-directory-fail",
    ] {
        let mut command = velune_agent_runtime::process::command(&node);
        command.args(["--input-type=module", "--eval", r#"
import {pathToFileURL} from 'node:url';
import {existsSync,writeFileSync} from 'node:fs';
const [sdk,binary,root,mode]=process.argv.slice(1);
const {resolvePiSdk}=await import(pathToFileURL(sdk).href);
const {SessionManager}=await resolvePiSdk({binary,runtimeTypeId:'pi-1.0'}).importModule('dist/core/session-manager.js');
const manager=SessionManager.create(root,root+'/sessions');
const image={type:'image',mimeType:'image/png',data:'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Zl1sAAAAASUVORK5CYII='};
let first,removed,compaction;
if(mode!=='empty') {
 first=manager.appendMessage({role:'user',content:[{type:'text',text:'retained original'},image],timestamp:1});
 manager.appendMessage({role:'assistant',content:[{type:'toolCall',id:'paired-tool',name:'read',arguments:{path:'synthetic'}}],api:'openai-responses',provider:'synthetic',model:'alpha',stopReason:'toolUse',timestamp:2});
 manager.appendMessage({role:'toolResult',toolCallId:'paired-tool',toolName:'read',content:[{type:'text',text:'synthetic tool result'}],isError:false,timestamp:3});
 manager.appendCustomEntry('synthetic.preserved',{payload:'native custom payload'});
 manager.appendMessage({role:'custom',customType:'synthetic-notice',content:'native custom message',display:true,timestamp:4});
 removed=manager.appendMessage({role:'user',content:'retained but later deleted',timestamp:5});
}
if(mode==='compaction-carry-edits') {
 manager.appendMessage({role:'system',content:'synthetic retained system checkpoint',timestamp:6});
 compaction=manager.appendCompaction('synthetic compaction display summary',removed,12);
}
const selected=manager.appendMessage({role:'user',content:'selected old input',timestamp:6});
if(mode==='carry-edits' || mode==='compaction-carry-edits') {
 manager.appendContextEdit(first,{content:[{type:'text',text:'retained edited'},image]});
 manager.appendContextEdit(removed,null);
}
const path=manager.getSessionFile();
// A user-only synthetic source may not have crossed the SDK persistence fence.
// Materialize only its own SDK-produced header/entries, never real history.
if(!existsSync(path)) writeFileSync(path,[manager.getHeader(),...manager.getEntries()].map(record=>JSON.stringify(record)).join('\n')+'\n',{flag:'wx',mode:0o600});
console.log(JSON.stringify({path,selected,compaction}));
"#]).arg(resources.join("pi_sdk.mjs")).arg(&binary).arg(&root).arg(mode);
        let value: Value = serde_json::from_slice(
            &ExecutionTarget::Local
                .output(command, None, Some(&root))
                .unwrap(),
        )
        .unwrap();
        let source = PathBuf::from(value["path"].as_str().unwrap());
        let before = std::fs::read(&source).unwrap();
        let expected_digest = format!("{:x}", Sha256::digest(&before));
        let count_targets = || std::fs::read_dir(root.join("sessions")).unwrap().count();
        let target_count = count_targets();
        if mode == "stale" {
            // An external writer after authorization but before helper's first read.
            std::fs::OpenOptions::new()
                .append(true)
                .open(&source)
                .unwrap()
                .write_all(b"\n")
                .unwrap();
            let error = history::fork_pi_session(
                &config,
                &source,
                PiForkBoundary::BeforeEditedEntry {
                    entry: value["selected"].as_str().unwrap(),
                    expected_digest: &expected_digest,
                },
            )
            .unwrap_err();
            assert_eq!(error.code(), "fork_rejected");
            assert_eq!(
                count_targets(),
                target_count,
                "stale authorization created a target"
            );
            continue;
        }
        let original_display = history::read_pi(&config, &source).unwrap();
        let selected_suffix = format!(":{}:0", value["selected"].as_str().unwrap());
        let original_messages = original_display["messages"].as_array().unwrap();
        let boundary = original_messages
            .iter()
            .position(|message| {
                message["id"]
                    .as_str()
                    .is_some_and(|id| id.ends_with(&selected_suffix))
            })
            .expect("selected native entry was absent from full display projection");
        let mut active_config = config.clone();
        if mode.starts_with("sync-") {
            use std::os::unix::fs::PermissionsExt;
            let preload = root.join("fail-sync.mjs");
            let script = format!(
                r#"
import fs from 'node:fs';
import {{syncBuiltinESMExports}} from 'node:module';
const mode={mode}, sessions={sessions};
const paths=new Map(), originalOpen=fs.openSync, originalClose=fs.closeSync, originalSync=fs.fsyncSync;
fs.openSync=function(path,...args) {{ const fd=originalOpen.call(this,path,...args); paths.set(fd,String(path)); return fd; }};
fs.closeSync=function(fd) {{ try {{ return originalClose.call(this,fd); }} finally {{ paths.delete(fd); }} }};
fs.fsyncSync=function(fd) {{
 const path=paths.get(fd) ?? '', caller=new Error().stack.split('\n')[2] ?? '';
 if(caller.includes('/pi_sessions.mjs:') && ((mode==='sync-file-fail' && path.startsWith(sessions+'/') && path.endsWith('.jsonl')) || (mode==='sync-directory-fail' && path===sessions))) throw new Error('synthetic target durability failure');
 return originalSync.call(this,fd);
}};
syncBuiltinESMExports();
"#,
                mode = serde_json::to_string(mode).unwrap(),
                sessions = serde_json::to_string(&root.join("sessions")).unwrap()
            );
            std::fs::write(&preload, script).unwrap();
            let wrapper = root.join("node-sync-failure");
            let quote = |path: &std::ffi::OsStr| {
                format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"))
            };
            std::fs::write(
                &wrapper,
                format!(
                    "#!/bin/sh\nexec {} --import {} \"$@\"\n",
                    quote(std::ffi::OsStr::new(&node)),
                    quote(preload.as_os_str())
                ),
            )
            .unwrap();
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o700)).unwrap();
            active_config.node_binary = Some(wrapper);
        }
        let result = history::fork_pi_session(
            &active_config,
            &source,
            PiForkBoundary::BeforeEditedEntry {
                entry: value["selected"].as_str().unwrap(),
                expected_digest: &expected_digest,
            },
        );
        if mode.starts_with("sync-") {
            let error = result.unwrap_err();
            assert_eq!(
                error.code(),
                "fork_created",
                "durability failure lost recovery identity"
            );
            assert_eq!(
                count_targets(),
                target_count + 1,
                "failed sync must retain its recovery target"
            );
            assert_eq!(std::fs::read(&source).unwrap(), before);
            continue;
        }
        let target = result.unwrap();
        assert_eq!(
            std::fs::read(&source).unwrap(),
            before,
            "fork modified the original native source"
        );
        let projection = history::read_pi(&config, &target).unwrap();
        let text = projection.to_string();
        assert!(!text.contains("selected old input"));
        let target_messages = projection["messages"].as_array().unwrap();
        assert_eq!(
            target_messages.len(),
            boundary,
            "fork lost or duplicated complete display prefix rows"
        );
        for (original, target) in original_messages[..boundary].iter().zip(target_messages) {
            // Only the official new session header identity may differ. Every
            // retained source entry and its ordinal, including compaction's
            // multiple display rows, must still identify the same payload.
            let source_identity = |message: &Value| {
                message["id"]
                    .as_str()
                    .unwrap()
                    .rsplitn(3, ':')
                    .take(2)
                    .collect::<Vec<_>>()
                    .join(":")
            };
            assert_eq!(source_identity(original), source_identity(target));
            let mut original = original.clone();
            let mut target = target.clone();
            original.as_object_mut().unwrap().remove("id");
            target.as_object_mut().unwrap().remove("id");
            assert_eq!(
                original, target,
                "native display payload changed across the fork"
            );
        }
        if mode == "compaction-carry-edits" {
            let ordinal = format!(":{}:1", value["compaction"].as_str().unwrap());
            assert!(
                target_messages.iter().any(|message| message["id"]
                    .as_str()
                    .is_some_and(|id| id.ends_with(&ordinal))),
                "compaction source-entry ordinal was lost"
            );
            assert!(text.contains("synthetic compaction display summary"));
        }
        if mode == "empty" {
            assert!(projection["messages"].as_array().unwrap().is_empty());
            let header: Value = serde_json::from_str(
                std::fs::read_to_string(&target)
                    .unwrap()
                    .lines()
                    .next()
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(header["type"], "session");
        } else {
            assert!(text.contains("paired-tool") && text.contains("synthetic tool result"));
            assert!(text.contains("image/png") && text.contains("iVBORw0KGgo"));
            assert!(text.contains("native custom message"));
            assert!(
                std::fs::read_to_string(&target)
                    .unwrap()
                    .contains("native custom payload")
            );
            assert!(text.contains(
                if matches!(mode, "carry-edits" | "compaction-carry-edits") {
                    "retained edited"
                } else {
                    "retained original"
                }
            ));
            assert_eq!(
                text.contains("retained but later deleted"),
                !matches!(mode, "carry-edits" | "compaction-carry-edits")
            );
        }
    }
    std::fs::remove_dir_all(root).unwrap();
}
