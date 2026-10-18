use super::*;
use std::cell::RefCell;
use std::os::unix::fs::symlink;
use std::sync::atomic::{AtomicUsize, Ordering};

static SEQ: AtomicUsize = AtomicUsize::new(0);

/// A throwaway home folder, removed on drop.
struct Home {
    root: PathBuf,
}

impl Home {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!(
            "toolport-skills-{}-{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // Canonical so paths compare equal to what the scanner resolves.
        let root = std::fs::canonicalize(&root).unwrap();
        Home { root }
    }

    fn env(&self) -> Env {
        Env::from_home(&self.root)
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    fn write(&self, rel: &str, text: &str) -> PathBuf {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, text).unwrap();
        path
    }

    fn skill(&self, rel_dir: &str, name: &str, description: &str) -> PathBuf {
        self.write(
            &format!("{rel_dir}/SKILL.md"),
            &format!("---\nname: {name}\ndescription: {description}\n---\n\n# {name}\n"),
        )
    }

    fn link(&self, rel: &str, target: &Path) {
        let path = self.path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        symlink(target, &path).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.path(rel)).unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

#[derive(Default)]
struct TestWriter {
    backups: RefCell<Vec<(String, PathBuf)>>,
    trashed: RefCell<Vec<PathBuf>>,
}

impl Writer for TestWriter {
    fn backup(&self, client: &str, path: &Path) -> Result<(), String> {
        self.backups.borrow_mut().push((client.into(), path.into()));
        Ok(())
    }
    fn write(&self, path: &Path, contents: &str) -> Result<(), String> {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, contents).map_err(|e| e.to_string())
    }
    fn remove_link(&self, path: &Path) -> Result<(), String> {
        std::fs::remove_file(path).map_err(|e| e.to_string())
    }
    fn trash(&self, path: &Path) -> Result<(), String> {
        self.trashed.borrow_mut().push(path.into());
        std::fs::remove_dir_all(path).map_err(|e| e.to_string())
    }
}

fn all(_: &str) -> bool {
    true
}

fn only<'a>(ids: &'a [&'a str]) -> impl Fn(&str) -> bool + 'a {
    move |id| ids.contains(&id)
}

fn skill<'a>(inv: &'a Inventory, name: &str) -> &'a Skill {
    inv.skills
        .iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("no skill {name}"))
}

fn state(inv: &Inventory, name: &str, agent: &str) -> Option<State> {
    skill(inv, name).states.get(agent).map(|s| s.state)
}

#[test]
fn shared_skill_reaches_each_client_through_its_own_folders() {
    let home = Home::new();
    let real = home.skill(".agents/skills/inbox", "inbox", "What needs me");
    home.link(".claude/skills/inbox", real.parent().unwrap());
    home.link(".codex/skills/inbox", real.parent().unwrap());

    let inv = scan(&home.env(), &all, None);
    let inbox = skill(&inv, "inbox");
    assert_eq!(inbox.source, Source::Yours);
    for agent in [
        "claude-code",
        "codex",
        "cursor",
        "github-copilot-cli",
        "opencode",
        "grok",
        "gemini-cli",
    ] {
        assert_eq!(state(&inv, "inbox", agent), Some(State::On), "{agent}");
    }
    assert_eq!(state(&inv, "inbox", "hermes"), None);
    // One record for the real file, with every path that reaches it.
    assert_eq!(inv.skills.iter().filter(|s| s.name == "inbox").count(), 1);
    assert!(inbox.links.contains(&"~/.claude/skills/inbox".to_string()));
    assert!(inbox.links.contains(&"~/.agents/skills/inbox".to_string()));
    // Cursor reaches the same file through three folders and loads it once.
    let via = &inbox.states["cursor"].via;
    assert!(
        via.contains("~/.claude/skills") && via.contains("~/.agents/skills"),
        "{via}"
    );
}

#[test]
fn native_switches_turn_skills_off_per_client() {
    let home = Home::new();
    let real = home.skill(".agents/skills/orca", "orca", "Orca");
    home.link(".claude/skills/orca", real.parent().unwrap());
    home.write(
        ".claude/settings.json",
        r#"{"skillOverrides": {"orca": "off"}}"#,
    );
    home.write(
        ".codex/config.toml",
        &format!(
            "[[skills.config]]\npath = \"{}\"\nenabled = false\n",
            home.path(".agents/skills/orca/SKILL.md").display()
        ),
    );
    home.write(".copilot/settings.json", r#"{"disabledSkills": ["orca"]}"#);
    home.write(
        ".gemini/settings.json",
        r#"{"skills": {"disabled": ["orca"]}}"#,
    );
    home.write(
        ".config/opencode/opencode.json",
        r#"{"permissions": [{"action": "*", "resource": "*", "effect": "allow"}, {"action": "skill", "resource": "orca", "effect": "deny"}]}"#,
    );
    home.write(".hermes/config.yaml", "skills:\n  disabled:\n    - orca\n");
    home.link(".hermes/skills/orca", real.parent().unwrap());
    home.write(".grok/config.toml", "[skills]\ndisabled = [\"orca\"]\n");
    home.write(
        ".config/crush/crush.json",
        r#"{"options": {"disabled_skills": ["orca"]}}"#,
    );
    home.write(
        ".qwen/settings.json",
        r#"{"skills": {"disabled": ["orca"]}}"#,
    );
    home.write(
        ".omp/agent/config.yml",
        "skills:\n  ignoredSkills: [orca]\n",
    );

    let inv = scan(&home.env(), &all, None);
    for agent in [
        "claude-code",
        "codex",
        "github-copilot-cli",
        "gemini-cli",
        "opencode",
        "hermes",
        "grok",
        "crush",
        "qwen-code",
        "omp",
    ] {
        assert_eq!(state(&inv, "orca", agent), Some(State::Off), "{agent}");
    }
    // Cursor has no switch and still loads it.
    assert_eq!(state(&inv, "orca", "cursor"), Some(State::On));
    let issue = inv.issues.iter().find(|i| i.id == "off-elsewhere").unwrap();
    assert!(issue.agents.contains(&"cursor".to_string()));
    assert!(
        issue.fix_label.is_none(),
        "only Cursor still loads it, and it has no switch"
    );
}

#[test]
fn claude_name_only_and_synced_account_skills() {
    let home = Home::new();
    home.skill(".claude/skills/synced/bucket-a/pdf", "pdf", "PDF files");
    home.skill(
        ".claude/skills/synced/bucket-a/computer-use",
        "computer-use",
        "Desktop",
    );
    home.write(
        ".claude/settings.json",
        r#"{"skillOverrides": {"pdf": "name-only", "computer-use": "off"}}"#,
    );
    let inv = scan(&home.env(), &all, None);
    assert_eq!(skill(&inv, "pdf").source, Source::Account);
    assert_eq!(state(&inv, "pdf", "claude-code"), Some(State::NameOnly));
    assert_eq!(state(&inv, "computer-use", "claude-code"), Some(State::Off));
    // Clients that read Claude's folder recursively ignore Claude's setting.
    assert_eq!(state(&inv, "computer-use", "opencode"), Some(State::On));
    assert_eq!(state(&inv, "computer-use", "cursor"), Some(State::On));
    assert_eq!(state(&inv, "computer-use", "grok"), Some(State::On));
    // Copilot CLI doesn't read Claude's folder.
    assert_eq!(state(&inv, "computer-use", "github-copilot-cli"), None);
    let issue = inv.issues.iter().find(|i| i.id == "off-elsewhere").unwrap();
    assert_eq!(issue.fix_label.as_deref(), Some("Turn off where possible"));
}

#[test]
fn same_name_from_two_files_is_shadowed_by_client_precedence() {
    let home = Home::new();
    home.skill(".grok/skills/review", "review", "Grok copy");
    home.skill(".agents/skills/review", "review", "Shared copy");
    let inv = scan(&home.env(), &only(&["grok", "cursor"]), None);
    let grok_copy = inv
        .skills
        .iter()
        .find(|s| s.description == "Grok copy")
        .unwrap();
    let shared = inv
        .skills
        .iter()
        .find(|s| s.description == "Shared copy")
        .unwrap();
    // Grok: its own folder comes first.
    assert_eq!(grok_copy.states["grok"].state, State::On);
    assert_eq!(shared.states["grok"].state, State::Shadowed);
    // Cursor: the later folder (~/.agents/skills) overwrites.
    assert_eq!(shared.states["cursor"].state, State::On);
    assert_eq!(grok_copy.states["cursor"].state, State::Shadowed);
    let clash = inv.issues.iter().find(|i| i.id == "clashes").unwrap();
    // Two keys, one readable name.
    assert_eq!(clash.skills.len(), 2);
    assert_eq!(clash.skill_names.len(), 1);
    assert!(!clash.skill_names[0].contains('/'));
}

#[test]
fn broken_links_are_reported_and_only_dangling_links_are_removed() {
    let home = Home::new();
    home.link(
        ".grok/skills/computer-use",
        &home.path(".agents/skills/computer-use"),
    );
    let ok = home.skill(".agents/skills/ok", "ok", "fine");
    home.link(".grok/skills/ok", ok.parent().unwrap());

    let inv = scan(&home.env(), &only(&["grok"]), None);
    let broken = inv
        .skills
        .iter()
        .find(|s| s.source == Source::Broken)
        .unwrap();
    assert_eq!(broken.name, "computer-use");
    assert_eq!(broken.states["grok"].state, State::Broken);
    assert!(inv.issues.iter().any(|i| i.id == "broken-links"));

    let writer = TestWriter::default();
    let removed = apply_fix(&home.env(), &inv, "broken-links", &writer).unwrap();
    assert_eq!(removed, 1);
    assert!(std::fs::symlink_metadata(home.path(".grok/skills/computer-use")).is_err());
    assert!(home.path(".grok/skills/ok/SKILL.md").is_file());
}

#[test]
fn codex_stale_entry_is_reported_and_removed_keeping_the_rest() {
    let home = Home::new();
    let keep = home.skill(".agents/skills/keep", "keep", "Keep");
    home.write(
        ".codex/config.toml",
        &format!(
            "model = \"x\" # my model\n\n[[skills.config]]\npath = \"{}\"\nenabled = false\n\n[[skills.config]]\npath = \"{}\"\nenabled = false\n",
            home.path(".agents/skills/gone/SKILL.md").display(),
            keep.display()
        ),
    );
    let inv = scan(&home.env(), &only(&["codex"]), None);
    assert_eq!(state(&inv, "keep", "codex"), Some(State::Off));
    assert!(inv.issues.iter().any(|i| i.id == "codex-stale"));
    let writer = TestWriter::default();
    assert_eq!(
        apply_fix(&home.env(), &inv, "codex-stale", &writer).unwrap(),
        1
    );
    let text = home.read(".codex/config.toml");
    assert!(text.contains("# my model"));
    assert!(!text.contains("gone"));
    assert!(text.contains("keep/SKILL.md"));
    assert_eq!(writer.backups.borrow()[0].0, "codex");
}

#[test]
fn set_state_writes_each_clients_own_setting() {
    let home = Home::new();
    let real = home.skill(".agents/skills/orca", "orca", "Orca");
    home.link(".claude/skills/orca", real.parent().unwrap());
    home.link(".hermes/skills/orca", real.parent().unwrap());
    home.write(".claude/settings.json", "{\n  \"theme\": \"dark\"\n}\n");
    home.write(
        ".config/opencode/opencode.json",
        "{\n  // keep me\n  \"permissions\": [{\"action\": \"*\", \"resource\": \"*\", \"effect\": \"allow\"}]\n}\n",
    );
    home.write(
        ".hermes/config.yaml",
        "# Hermes\nskills:\n  creation_nudge_interval: 0\nmodel: x\n",
    );
    home.write(".grok/config.toml", "# mine\nmodel = \"grok-5\"\n");
    home.write(".omp/agent/config.yml", "# Oh My Pi\ntheme: dark\n");
    let env = home.env();
    let writer = TestWriter::default();
    let inv = scan(&env, &all, None);
    let key = skill(&inv, "orca").key.clone();
    for agent in [
        "claude-code",
        "codex",
        "github-copilot-cli",
        "gemini-cli",
        "opencode",
        "hermes",
        "grok",
        "crush",
        "qwen-code",
        "omp",
    ] {
        set_state(&env, &inv, agent, &key, Wanted::Off, &writer).unwrap();
    }
    let after = scan(&env, &all, None);
    for agent in [
        "claude-code",
        "codex",
        "github-copilot-cli",
        "gemini-cli",
        "opencode",
        "hermes",
        "grok",
        "crush",
        "qwen-code",
        "omp",
    ] {
        assert_eq!(state(&after, "orca", agent), Some(State::Off), "{agent}");
    }
    assert!(home
        .read(".claude/settings.json")
        .contains("\"theme\": \"dark\""));
    assert!(home
        .read(".config/opencode/opencode.json")
        .contains("// keep me"));
    let hermes = home.read(".hermes/config.yaml");
    assert!(
        hermes.starts_with(
            "# Hermes\nskills:\n  disabled: [\"orca\"]\n  creation_nudge_interval: 0\nmodel: x"
        ),
        "{hermes}"
    );
    let grok = home.read(".grok/config.toml");
    assert!(
        grok.starts_with("# mine\nmodel = \"grok-5\"\n") && grok.contains("disabled = [\"orca\"]"),
        "{grok}"
    );
    assert!(home
        .read(".omp/agent/config.yml")
        .starts_with("# Oh My Pi\ntheme: dark\nskills:\n  ignoredSkills: [\"orca\"]"));
    assert!(home
        .read(".config/crush/crush.json")
        .contains("disabled_skills"));
    // Codex gets the path it loads the skill through.
    assert!(home.read(".codex/config.toml").contains(
        &home
            .path(".agents/skills/orca/SKILL.md")
            .display()
            .to_string()
    ));

    // And back on.
    for agent in [
        "claude-code",
        "codex",
        "github-copilot-cli",
        "gemini-cli",
        "opencode",
        "hermes",
        "grok",
        "crush",
        "qwen-code",
        "omp",
    ] {
        set_state(&env, &after, agent, &key, Wanted::On, &writer).unwrap();
    }
    let back = scan(&env, &all, None);
    for agent in [
        "claude-code",
        "codex",
        "github-copilot-cli",
        "gemini-cli",
        "opencode",
        "hermes",
        "grok",
        "crush",
        "qwen-code",
        "omp",
    ] {
        assert_eq!(state(&back, "orca", agent), Some(State::On), "{agent}");
    }
    assert!(!home.read(".codex/config.toml").contains("orca"));
    assert!(!home.read(".grok/config.toml").contains("orca"));
    assert!(!home.read(".omp/agent/config.yml").contains("orca"));
    // Existing files were backed up before each first change; new files were not.
    let backups = writer.backups.borrow();
    assert!(backups.iter().any(|(c, _)| c == "claude-code"));
    assert!(backups.iter().any(|(c, _)| c == "hermes"));
    assert!(backups.iter().any(|(c, _)| c == "grok"));
}

#[test]
fn set_state_refuses_clients_without_a_switch_and_name_only_outside_claude() {
    let home = Home::new();
    home.skill(".agents/skills/a", "a", "A");
    let env = home.env();
    let inv = scan(&env, &all, None);
    let key = skill(&inv, "a").key.clone();
    let writer = TestWriter::default();
    assert!(set_state(&env, &inv, "cursor", &key, Wanted::Off, &writer).is_err());
    assert!(set_state(&env, &inv, "codex", &key, Wanted::NameOnly, &writer).is_err());
    assert!(
        set_state(&env, &inv, "hermes", &key, Wanted::Off, &writer).is_err(),
        "Hermes doesn't load it"
    );
}

#[test]
fn opencode_v1_permission_map_is_read_and_written_in_place() {
    let mut config = serde_json::json!({"permission": {"skill": {"a": "deny"}}});
    assert!(opencode_denied(&config).contains("a"));
    opencode_toggle(&mut config, "a", false).unwrap();
    opencode_toggle(&mut config, "b", true).unwrap();
    assert!(config.get("permissions").is_none());
    assert_eq!(opencode_denied(&config), BTreeSet::from(["b".to_string()]));
    // v2: the last matching rule wins.
    let v2 = serde_json::json!({"permissions": [
        {"action": "skill", "resource": "a", "effect": "deny"},
        {"action": "skill", "resource": "a", "effect": "allow"}
    ]});
    assert!(opencode_denied(&v2).is_empty());
}

fn set_yaml_skills_list_disabled(text: &str, names: &[String]) -> Result<String, String> {
    set_yaml_skills_list(text, "disabled", names)
}

#[test]
fn yaml_list_edits_touch_only_their_own_key() {
    let names = vec!["a".to_string()];
    assert_eq!(
        set_yaml_skills_list("skills:\n  disabled: [x]\n", "ignoredSkills", &names).unwrap(),
        "skills:\n  ignoredSkills: [\"a\"]\n  disabled: [x]\n"
    );
}

#[test]
fn yaml_disabled_list_edits_keep_other_lines() {
    let names = vec!["a".to_string(), "b".to_string()];
    assert_eq!(
        set_yaml_skills_list_disabled("", &names).unwrap(),
        "skills:\n  disabled: [\"a\", \"b\"]\n"
    );
    assert_eq!(
        set_yaml_skills_list_disabled("model: x\n", &names).unwrap(),
        "model: x\nskills:\n  disabled: [\"a\", \"b\"]\n"
    );
    assert_eq!(
        set_yaml_skills_list_disabled(
            "skills:\n    disabled:\n    - old\n    - older\n    other: 1\nnext: 2\n",
            &names
        )
        .unwrap(),
        "skills:\n    disabled: [\"a\", \"b\"]\n    other: 1\nnext: 2\n"
    );
    assert_eq!(
        set_yaml_skills_list_disabled("skills:\n  disabled: [x]\n", &[]).unwrap(),
        "skills:\n  disabled: []\n"
    );
}

#[test]
fn rules_follow_links_and_imports_and_note_skipped_imports() {
    let home = Home::new();
    let agreements = home.write(".agents/working-agreements.md", "# Working agreements\n");
    let instructions = home.write(
        ".agents/claude-instructions.md",
        &format!("@{}\n\n# Delegation\n", agreements.display()),
    );
    home.link(".claude/CLAUDE.md", &instructions);
    home.link(".codex/AGENTS.md", &agreements);
    std::fs::create_dir_all(home.path(".grok")).unwrap();

    let inv = scan(
        &home.env(),
        &only(&["claude-code", "codex", "grok", "github-copilot-cli"]),
        None,
    );
    let wa = inv
        .rules
        .iter()
        .find(|r| r.name == "working-agreements.md")
        .unwrap();
    let chain: Vec<(String, String)> = wa.agents["claude-code"]
        .iter()
        .map(|c| (c.kind.clone(), c.path.clone()))
        .collect();
    assert_eq!(
        chain,
        vec![
            ("file".to_string(), "~/.claude/CLAUDE.md".to_string()),
            (
                "link".to_string(),
                "~/.agents/claude-instructions.md".to_string()
            ),
            (
                "import".to_string(),
                "~/.agents/working-agreements.md".to_string()
            ),
        ]
    );
    assert!(wa.agents.contains_key("codex"));
    assert!(!wa.agents.contains_key("grok"));
    let ci = inv
        .rules
        .iter()
        .find(|r| r.name == "claude-instructions.md")
        .unwrap();
    assert!(ci.agents["grok"]
        .iter()
        .any(|c| c.kind == "skipped-import" && c.path == "~/.agents/working-agreements.md"));
    assert!(inv.issues.iter().any(|i| i.id == "rules-imports"));
    let missing = inv.issues.iter().find(|i| i.id == "rules-missing").unwrap();
    assert_eq!(missing.agents, vec!["github-copilot-cli".to_string()]);
}

#[test]
fn codex_override_file_hides_agents_md() {
    let home = Home::new();
    home.write(".codex/AGENTS.md", "base\n");
    home.write(".codex/AGENTS.override.md", "override\n");
    let inv = scan(&home.env(), &only(&["codex"]), None);
    assert!(inv.rules.iter().any(|r| r.name == "AGENTS.override.md"));
    assert!(!inv.rules.iter().any(|r| r.name == "AGENTS.md"));
}

#[test]
fn project_scope_adds_project_skills_and_rules() {
    let home = Home::new();
    home.skill("work/app/.claude/skills/deploy", "deploy", "Deploy the app");
    home.write("work/app/AGENTS.md", "# App\n");
    home.write("work/app/CLAUDE.md", "@AGENTS.md\n");
    let project = home.path("work/app");
    let inv = scan(
        &home.env(),
        &only(&["claude-code", "codex"]),
        Some(&project),
    );
    assert_eq!(state(&inv, "deploy", "claude-code"), Some(State::On));
    let agents_md = inv.rules.iter().find(|r| r.name == "AGENTS.md").unwrap();
    assert_eq!(agents_md.scope, "project");
    assert!(agents_md.agents["claude-code"]
        .iter()
        .any(|c| c.kind == "import"));
    assert!(agents_md.agents.contains_key("codex"));
}

#[test]
fn parked_package_and_builtin_sources() {
    let home = Home::new();
    home.skill(".agents/disabled-skills/ds", "ds", "Parked");
    home.skill(".codex/skills/.system/imagegen", "imagegen", "Images");
    home.skill(".grok/bundled/skills/review", "review", "Review");
    home.skill(".hermes/skills/devops/build-host", "build-host", "Tuning");
    let inv = scan(&home.env(), &all, None);
    assert_eq!(skill(&inv, "ds").source, Source::Parked);
    assert!(skill(&inv, "ds").states.is_empty());
    assert_eq!(skill(&inv, "imagegen").source, Source::BuiltIn);
    assert_eq!(state(&inv, "imagegen", "codex"), Some(State::On));
    // Cursor skips Codex's hidden .system folder.
    assert_eq!(state(&inv, "imagegen", "cursor"), None);
    assert_eq!(skill(&inv, "review").source, Source::BuiltIn);
    assert_eq!(skill(&inv, "build-host").source, Source::Library);
    assert_eq!(state(&inv, "build-host", "hermes"), Some(State::On));
}

#[test]
fn long_descriptions_and_front_matter() {
    let (name, description) =
        front_matter("---\nname: x\ndescription: >\n  folded\n  text\n---\nbody");
    assert_eq!(name.as_deref(), Some("x"));
    assert_eq!(description.as_deref(), Some("folded text"));
    assert_eq!(front_matter("no front matter"), (None, None));

    let home = Home::new();
    home.skill(
        ".claude/skills/big",
        "big",
        &"y".repeat(DESCRIPTION_CAP + 1),
    );
    let inv = scan(&home.env(), &only(&["claude-code"]), None);
    assert!(inv.issues.iter().any(|i| i.id == "long-descriptions"));
}

#[test]
fn clients_without_skills_are_listed_but_not_scanned() {
    let home = Home::new();
    let inv = scan(&home.env(), &only(&["claude-desktop"]), None);
    assert_eq!(inv.agents.len(), 1);
    assert!(!inv.agents[0].supports_skills);
    assert!(inv.skills.is_empty());
}

#[test]
fn allowed_paths_are_limited_to_the_inventory() {
    let home = Home::new();
    let real = home.skill(".agents/skills/a", "a", "A");
    let inv = scan(&home.env(), &all, None);
    assert!(allowed_path(&inv, &real.display().to_string()));
    assert!(allowed_path(
        &inv,
        &real.parent().unwrap().display().to_string()
    ));
    assert!(!allowed_path(&inv, "/etc/passwd"));
}

/// Manual check against a real machine: `cargo test --lib skills::tests::dump_real_inventory --
/// --ignored --nocapture`. Read-only.
#[test]
#[ignore]
fn dump_real_inventory() {
    let inv = scan(&Env::current().unwrap(), &|_| true, None);
    println!("{}", serde_json::to_string_pretty(&inv).unwrap());
}

#[test]
fn other_clients_read_the_default_claude_folder_not_claude_config_dir() {
    let home = Home::new();
    home.skill(".claude/skills/work", "work", "Default folder");
    home.skill(
        ".claude-personal/skills/personal",
        "personal",
        "Relocated folder",
    );
    let mut env = home.env();
    env.claude = home.path(".claude-personal");
    let inv = scan(&env, &only(&["claude-code", "cursor"]), None);
    assert_eq!(state(&inv, "personal", "claude-code"), Some(State::On));
    assert_eq!(state(&inv, "personal", "cursor"), None);
    assert_eq!(state(&inv, "work", "cursor"), Some(State::On));
    assert_eq!(state(&inv, "work", "claude-code"), None);
}

#[test]
fn broken_links_do_not_shadow_a_working_skill_with_the_same_name() {
    let home = Home::new();
    home.link(
        ".grok/skills/computer-use",
        &home.path(".agents/skills/computer-use"),
    );
    home.skill(
        ".claude/skills/synced/b/computer-use",
        "computer-use",
        "Desktop",
    );
    let inv = scan(&home.env(), &only(&["grok"]), None);
    let working = inv
        .skills
        .iter()
        .find(|s| s.source == Source::Account)
        .unwrap();
    assert_eq!(working.states["grok"].state, State::On);
    assert!(!inv.issues.iter().any(|i| i.id == "clashes"));
}

#[test]
fn front_matter_that_isnt_valid_yaml_still_gives_name_and_description() {
    let (name, description) = front_matter(
        "---\nname: deploy-check\ndescription: Check a deploy: health and errors\n---\n",
    );
    assert_eq!(name.as_deref(), Some("deploy-check"));
    assert_eq!(
        description.as_deref(),
        Some("Check a deploy: health and errors")
    );
    let (_, folded) =
        front_matter("---\nname: x\ndescription: >\n  First line: more\n  second\nother: 1\n---\n");
    assert_eq!(folded.as_deref(), Some("First line: more second"));
}

#[test]
fn opencode_wildcards_decide_state_and_writes_never_widen_a_blanket_rule() {
    // v2: a later wildcard deny turns everything off; an explicit allow after it wins.
    let v2 = serde_json::json!({"permissions": [
        {"action": "skill", "resource": "keep", "effect": "allow"},
        {"action": "skill", "resource": "*", "effect": "deny"}
    ]});
    let rules = opencode_rules(&v2);
    assert!(rules_deny(&rules, "keep"));
    assert!(rules_deny(&rules, "anything"));
    let mut config = v2.clone();
    opencode_toggle(&mut config, "keep", false).unwrap();
    assert!(!rules_deny(&opencode_rules(&config), "keep"));
    assert!(rules_deny(&opencode_rules(&config), "other"));
    // Patterns match by glob.
    assert!(glob_match("orca-*", "orca-cli"));
    assert!(!glob_match("orca-*", "inbox"));
    // v1: one string for every skill is refused, never replaced by a single-skill map.
    let mut blanket = serde_json::json!({"permission": {"skill": "deny"}});
    assert!(opencode_toggle(&mut blanket, "x", true).is_err());
    assert_eq!(
        blanket,
        serde_json::json!({"permission": {"skill": "deny"}})
    );
    // v1 map with a wildcard: turning one on adds an allow instead of only removing its entry.
    let mut map = serde_json::json!({"permission": {"skill": {"*": "deny", "x": "deny"}}});
    opencode_toggle(&mut map, "x", false).unwrap();
    assert_eq!(map["permission"]["skill"]["x"], "allow");
    assert!(rules_deny(&opencode_rules(&map), "y"));
}

#[test]
fn opencode_permission_shorthand_is_spelled_out_not_refused() {
    // `"permission": "allow"` is what OpenCode writes for "allow everything".
    let mut config = serde_json::json!({"permission": "allow", "mcp": {}});
    opencode_toggle(&mut config, "orca-cli", true).unwrap();
    assert_eq!(
        config,
        serde_json::json!({
            "permission": {"*": "allow", "skill": {"orca-cli": "deny"}},
            "mcp": {}
        })
    );
    assert_eq!(
        opencode_denied(&config),
        BTreeSet::from(["orca-cli".to_string()])
    );
    // A blanket deny turns every skill off; turning one back on adds an explicit allow.
    let mut deny_all = serde_json::json!({"permission": "deny"});
    assert!(rules_deny(&opencode_rules(&deny_all), "inbox"));
    opencode_toggle(&mut deny_all, "inbox", false).unwrap();
    assert!(!rules_deny(&opencode_rules(&deny_all), "inbox"));
    assert!(rules_deny(&opencode_rules(&deny_all), "other"));
    // v2 all-actions rules reach skills only when they cover every resource.
    let v2 = serde_json::json!({"permissions": [
        {"action": "*", "resource": "*", "effect": "deny"},
        {"action": "*", "resource": "inbox", "effect": "allow"}
    ]});
    assert!(rules_deny(&opencode_rules(&v2), "inbox"));
}

#[test]
fn opencode_permission_shorthand_scans_and_toggles_end_to_end() {
    let home = Home::new();
    home.skill(".agents/skills/inbox", "inbox", "Inbox");
    home.write(
        ".config/opencode/opencode.json",
        r#"{"permission": "allow", "mcp": {"toolport": {"type": "local"}}}"#,
    );
    let inv = scan(&home.env(), &only(&["opencode"]), None);
    assert_eq!(state(&inv, "inbox", "opencode"), Some(State::On));
    let key = skill(&inv, "inbox").key.clone();
    set_state(
        &home.env(),
        &inv,
        "opencode",
        &key,
        Wanted::Off,
        &TestWriter::default(),
    )
    .unwrap();
    let inv = scan(&home.env(), &only(&["opencode"]), None);
    assert_eq!(state(&inv, "inbox", "opencode"), Some(State::Off));
    let saved: serde_json::Value =
        serde_json::from_str(&home.read(".config/opencode/opencode.json")).unwrap();
    assert_eq!(saved["permission"]["*"], "allow");
    assert_eq!(saved["mcp"]["toolport"]["type"], "local");
}

#[test]
fn opencode_wildcard_deny_shows_skills_off() {
    let home = Home::new();
    home.skill(".agents/skills/inbox", "inbox", "Inbox");
    home.write(
        ".config/opencode/opencode.json",
        r#"{"permissions": [{"action": "skill", "resource": "*", "effect": "deny"}]}"#,
    );
    let inv = scan(&home.env(), &only(&["opencode"]), None);
    assert_eq!(state(&inv, "inbox", "opencode"), Some(State::Off));
}

#[test]
fn json_settings_in_an_unexpected_shape_are_refused_not_replaced() {
    let home = Home::new();
    home.skill(".agents/skills/inbox", "inbox", "Inbox");
    home.write(".copilot/settings.json", r#"{"disabledSkills": "inbox"}"#);
    let env = home.env();
    let inv = scan(&env, &only(&["github-copilot-cli"]), None);
    let key = skill(&inv, "inbox").key.clone();
    let writer = TestWriter::default();
    assert!(set_state(&env, &inv, "github-copilot-cli", &key, Wanted::Off, &writer).is_err());
    assert_eq!(
        home.read(".copilot/settings.json"),
        r#"{"disabledSkills": "inbox"}"#
    );
}

#[test]
fn hermes_edits_keep_comments_and_refuse_what_they_cannot_read() {
    let names = vec!["a".to_string()];
    // A comment after the key is still the same key, not a reason to add a second one.
    assert_eq!(
        set_yaml_skills_list_disabled("skills: # mine\n  other: 1\n", &names).unwrap(),
        "skills: # mine\n  disabled: [\"a\"]\n  other: 1\n"
    );
    // A flow mapping on the key's line is refused.
    assert!(set_yaml_skills_list_disabled("skills: {disabled: [b]}\n", &names).is_err());

    // A config serde_yaml can't parse keeps its list: the toggle fails instead.
    let home = Home::new();
    home.skill(".hermes/skills/notes", "notes", "Notes");
    let broken = "skills:\n  disabled: [x]\nmodel: a\nmodel: b\n";
    home.write(".hermes/config.yaml", broken);
    let env = home.env();
    let inv = scan(&env, &only(&["hermes"]), None);
    let key = skill(&inv, "notes").key.clone();
    let writer = TestWriter::default();
    assert!(set_state(&env, &inv, "hermes", &key, Wanted::Off, &writer).is_err());
    assert_eq!(home.read(".hermes/config.yaml"), broken);
}

#[test]
fn codex_turn_on_clears_entries_written_as_the_folder_or_real_path() {
    let home = Home::new();
    let real = home.skill(".agents/skills/inbox", "inbox", "Inbox");
    let folder = real.parent().unwrap().display().to_string();
    home.write(
        ".codex/config.toml",
        &format!("[[skills.config]]\npath = \"{folder}\"\nenabled = false\n"),
    );
    let env = home.env();
    let inv = scan(&env, &only(&["codex"]), None);
    assert_eq!(state(&inv, "inbox", "codex"), Some(State::Off));
    let key = skill(&inv, "inbox").key.clone();
    let writer = TestWriter::default();
    set_state(&env, &inv, "codex", &key, Wanted::On, &writer).unwrap();
    let inv = scan(&env, &only(&["codex"]), None);
    assert_eq!(state(&inv, "inbox", "codex"), Some(State::On));
}

#[test]
fn unreadable_codex_config_is_an_error_not_an_empty_file() {
    let home = Home::new();
    home.skill(".agents/skills/inbox", "inbox", "Inbox");
    home.write(".codex/config.toml", "model = \"x\"\n");
    let env = home.env();
    let inv = scan(&env, &only(&["codex"]), None);
    let key = skill(&inv, "inbox").key.clone();
    // Too big to read: the write must fail rather than replace the file.
    let big = format!(
        "model = \"x\"\n# {}\n",
        "x".repeat(MAX_FILE_BYTES as usize + 1)
    );
    home.write(".codex/config.toml", &big);
    let writer = TestWriter::default();
    assert!(set_state(&env, &inv, "codex", &key, Wanted::Off, &writer).is_err());
    assert_eq!(home.read(".codex/config.toml").len(), big.len());
}

#[test]
fn claude_plugin_skills_switch_by_plugin_name_and_dont_clash_with_yours() {
    let home = Home::new();
    home.write(
        ".claude/settings.json",
        r#"{"enabledPlugins": {"tools@market": true}}"#,
    );
    home.skill(
        ".claude/plugins/cache/market/tools/1.0.0/skills/review",
        "review",
        "Plugin review",
    );
    home.skill(".claude/skills/review", "review", "My review");
    let env = home.env();
    let inv = scan(&env, &only(&["claude-code"]), None);
    let mine = inv
        .skills
        .iter()
        .find(|s| s.description == "My review")
        .unwrap();
    let plugin = inv
        .skills
        .iter()
        .find(|s| s.description == "Plugin review")
        .unwrap();
    assert_eq!(mine.states["claude-code"].state, State::On);
    assert_eq!(plugin.states["claude-code"].state, State::On);
    assert!(!inv.issues.iter().any(|i| i.id == "clashes"));
    let writer = TestWriter::default();
    set_state(&env, &inv, "claude-code", &plugin.key, Wanted::Off, &writer).unwrap();
    let settings = home.read(".claude/settings.json");
    assert!(settings.contains(r#""tools:review": "off""#), "{settings}");
    let inv = scan(&env, &only(&["claude-code"]), None);
    let mine = inv
        .skills
        .iter()
        .find(|s| s.description == "My review")
        .unwrap();
    assert_eq!(mine.states["claude-code"].state, State::On);
}

#[test]
fn a_fix_that_edits_one_file_many_times_backs_it_up_once() {
    let home = Home::new();
    for name in ["a", "b", "c", "d", "e", "f", "g"] {
        home.skill(&format!(".agents/skills/{name}"), name, "Skill");
        home.link(
            &format!(".claude/skills/{name}"),
            &home.path(&format!(".agents/skills/{name}")),
        );
    }
    home.write(
        ".claude/settings.json",
        r#"{"skillOverrides": {"a": "off", "b": "off", "c": "off", "d": "off", "e": "off", "f": "off", "g": "off"}}"#,
    );
    home.write(".copilot/settings.json", "{}");
    let env = home.env();
    let inv = scan(&env, &only(&["claude-code", "github-copilot-cli"]), None);
    let writer = TestWriter::default();
    assert_eq!(apply_fix(&env, &inv, "off-elsewhere", &writer).unwrap(), 7);
    let backups = writer.backups.borrow();
    assert_eq!(backups.len(), 1, "{backups:?}");
    let inv = scan(&env, &only(&["claude-code", "github-copilot-cli"]), None);
    assert_eq!(state(&inv, "g", "github-copilot-cli"), Some(State::Off));
}

#[test]
fn skills_count_as_yours_through_a_symlinked_home_or_inside_the_project() {
    let home = Home::new();
    home.skill("real-home/.agents/skills/inbox", "inbox", "Inbox");
    home.link("home", &home.path("real-home"));
    let env = Env::from_home(&home.path("home"));
    let inv = scan(&env, &only(&["codex"]), None);
    let inbox = skill(&inv, "inbox");
    assert_eq!(inbox.source, Source::Yours);
    assert!(inbox.path.starts_with("~/"), "{}", inbox.path);

    let project = home.path("work/repo");
    home.skill("work/repo/.agents/skills/build", "build", "Build");
    let other = Home::new();
    let inv = scan(&other.env(), &only(&["codex"]), Some(&project));
    assert_eq!(skill(&inv, "build").source, Source::Yours);
}

#[test]
fn only_dangling_links_count_as_broken() {
    let home = Home::new();
    let target = home.skill(".agents/skills/inbox", "inbox", "Inbox");
    home.link(".grok/skills/inbox", target.parent().unwrap());
    assert!(!dangling(&home.path(".grok/skills/inbox")));
    home.link(".grok/skills/gone", &home.path(".agents/skills/gone"));
    assert!(dangling(&home.path(".grok/skills/gone")));
    // A plain folder is never removed as a link.
    assert!(DiskWriter
        .remove_link(&home.path(".agents/skills/inbox"))
        .is_err());
    assert!(home.path(".agents/skills/inbox").is_dir());
}

#[test]
fn open_never_runs_a_non_text_file_it_reached_through_an_import() {
    assert!(opens_as_text(Path::new("/p/CLAUDE.md")));
    assert!(opens_as_text(Path::new("/p/rules.MDC")));
    assert!(!opens_as_text(Path::new("/p/run.sh")));
    assert!(!opens_as_text(Path::new("/p/setup.bat")));
    assert!(!opens_as_text(Path::new("/p/no-extension")));
}

#[test]
fn lists_join_with_localized_separators() {
    let names: Vec<String> = ["a", "b", "c"].iter().map(|s| s.to_string()).collect();
    assert_eq!(list_names(&names), "a, b and c");
    assert_eq!(list_names(&names[..2]), "a and b");
}

#[test]
fn removing_a_skill_trashes_its_folder_and_every_link_to_it() {
    let home = Home::new();
    let real = home.skill(".agents/skills/orca-cli", "orca-cli", "Orca");
    let folder = real.parent().unwrap().to_path_buf();
    home.link(".claude/skills/orca-cli", &folder);
    home.skill(".agents/skills/inbox", "inbox", "Inbox");
    let env = home.env();
    let inv = scan(&env, &all, None);
    let key = skill(&inv, "orca-cli").key.clone();
    let writer = TestWriter::default();

    let (trashed, links) = remove_skill_files(&env, &inv, &key, &writer).unwrap();

    assert_eq!(trashed.as_deref(), Some(folder.as_path()));
    assert_eq!(links, 1);
    assert_eq!(*writer.trashed.borrow(), vec![folder.clone()]);
    assert!(!is_link(&home.path(".claude/skills/orca-cli")));
    let after = scan(&env, &all, None);
    assert!(after.skills.iter().all(|s| s.name != "orca-cli"));
    // Nothing is left behind as a broken link, and other skills stay.
    assert!(after.issues.iter().all(|i| i.id != "broken-links"));
    assert!(after.skills.iter().any(|s| s.name == "inbox"));
}

#[test]
fn removing_a_linked_skill_keeps_the_folder_it_points_at() {
    let home = Home::new();
    let real = home.skill("Projects/omabox/skill", "omabox", "Boxes");
    let folder = real.parent().unwrap().to_path_buf();
    home.link(".agents/skills/omabox", &folder);
    let env = home.env();
    let inv = scan(&env, &all, None);
    let key = skill(&inv, "omabox").key.clone();
    let writer = TestWriter::default();

    let (trashed, links) = remove_skill_files(&env, &inv, &key, &writer).unwrap();

    assert_eq!(trashed, None);
    assert_eq!(links, 1);
    assert!(writer.trashed.borrow().is_empty());
    assert!(real.is_file(), "the repo's folder is untouched");
    assert!(!is_link(&home.path(".agents/skills/omabox")));
}

#[test]
fn removing_a_skill_inside_a_linked_folder_touches_nothing() {
    let home = Home::new();
    // A skills folder entry that links to a projects folder with skills further down.
    let real = home.skill("work/project-a", "project-a", "A repo");
    home.write("work/project-a/src/main.rs", "fn main() {}\n");
    home.link(".claude/skills/work", &home.path("work"));
    let env = home.env();
    let inv = scan(&env, &all, None);
    let key = skill(&inv, "project-a").key.clone();
    let writer = TestWriter::default();

    let error = remove_skill_files(&env, &inv, &key, &writer).unwrap_err();

    assert!(error.contains("linked folder"), "{error}");
    assert!(writer.trashed.borrow().is_empty());
    assert!(real.is_file());
    assert!(is_link(&home.path(".claude/skills/work")));
}

#[test]
fn a_skills_folder_that_is_itself_a_link_still_counts_as_in_place() {
    let home = Home::new();
    let real = home.skill("dotfiles/skills/notes", "notes", "Notes");
    home.link(".agents/skills", &home.path("dotfiles/skills"));
    let env = home.env();
    let inv = scan(&env, &all, None);
    let key = skill(&inv, "notes").key.clone();
    let writer = TestWriter::default();

    let (trashed, _) = remove_skill_files(&env, &inv, &key, &writer).unwrap();

    assert_eq!(trashed.as_deref(), real.parent());
}

#[test]
fn each_claude_code_profile_gets_its_own_column_and_settings() {
    let home = Home::new();
    home.skill(".claude/skills/work-notes", "work-notes", "Work");
    home.write(".claude2/settings.json", "{}\n");
    home.skill(".claude2/skills/home-notes", "home-notes", "Home");
    home.write(".claude2/CLAUDE.md", "# Personal\n");
    // Not profiles: a backup, and another tool's folder with no Claude Code files.
    home.write(".claude-backup/settings.json", "{}\n");
    home.skill(".claude-router/skills/x", "x", "X");
    let mut env = home.env();
    env.claude_profiles = claude_profile_dirs(&home.root, &env.claude);
    assert_eq!(
        env.claude_profiles,
        vec![home.path(".claude"), home.path(".claude2")]
    );

    let inv = scan(&env, &only(&["claude-code", "cursor"]), None);
    let names: Vec<(&str, &str)> = inv
        .agents
        .iter()
        .map(|a| (a.id.as_str(), a.name.as_str()))
        .collect();
    assert_eq!(
        names,
        vec![
            ("claude-code", "Claude Code (claude)"),
            ("claude-code@claude2", "Claude Code (claude2)"),
            ("cursor", "Cursor"),
        ]
    );
    assert!(inv.agents.iter().filter(|a| a.name_only).count() == 2);
    let home_notes = skill(&inv, "home-notes");
    assert_eq!(
        home_notes.states.keys().collect::<Vec<_>>(),
        vec!["claude-code@claude2"]
    );
    // Cursor reads only ~/.claude, never the other profiles.
    assert!(skill(&inv, "work-notes").states.contains_key("cursor"));
    let personal = inv
        .rules
        .iter()
        .find(|r| r.path == "~/.claude2/CLAUDE.md")
        .unwrap();
    assert_eq!(
        personal.agents.keys().collect::<Vec<_>>(),
        vec!["claude-code@claude2"]
    );

    let key = home_notes.key.clone();
    set_state(
        &env,
        &inv,
        "claude-code@claude2",
        &key,
        Wanted::Off,
        &TestWriter::default(),
    )
    .unwrap();
    assert!(home.read(".claude2/settings.json").contains("home-notes"));
    assert!(!home.path(".claude/settings.json").exists());
    let after = scan(&env, &only(&["claude-code", "cursor"]), None);
    assert_eq!(
        state(&after, "home-notes", "claude-code@claude2"),
        Some(State::Off)
    );
}

#[test]
fn profiles_whose_folders_share_a_name_get_different_columns() {
    let home = Home::new();
    home.write(".claude/settings.json", "{}\n");
    home.write(".claude2/settings.json", "{}\n");
    home.write("data/.claude2/settings.json", "{}\n");
    let mut env = home.env();
    env.claude = home.path("data/.claude2");
    env.claude_profiles = claude_profile_dirs(&home.root, &env.claude);
    assert_eq!(env.claude_profiles.len(), 3);

    let inv = scan(&env, &only(&["claude-code"]), None);
    let names: Vec<&str> = inv.agents.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Claude Code (claude)",
            "Claude Code (~/data/.claude2)",
            "Claude Code (~/.claude2)",
        ]
    );
    let ids: BTreeSet<&str> = inv.agents.iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids.len(), 3);
    // Ids name a backup folder, so they hold no path separators.
    assert!(
        ids.iter().all(|id| !id.contains(['/', '\\', '~'])),
        "{ids:?}"
    );
}

#[test]
fn one_claude_code_profile_keeps_the_plain_column() {
    let home = Home::new();
    home.skill(".claude/skills/notes", "notes", "Notes");
    let mut env = home.env();
    env.claude_profiles = claude_profile_dirs(&home.root, &env.claude);
    assert!(env.claude_profiles.is_empty());
    let inv = scan(&env, &only(&["claude-code"]), None);
    assert_eq!(inv.agents[0].id, "claude-code");
    assert_eq!(inv.agents[0].name, "Claude Code");
}

#[test]
fn a_blanket_opencode_deny_is_not_a_choice_to_spread() {
    let home = Home::new();
    home.skill(".agents/skills/inbox", "inbox", "Inbox");
    home.skill(".agents/skills/notes", "notes", "Notes");
    home.link(".claude/skills/inbox", &home.path(".agents/skills/inbox"));
    home.link(".claude/skills/notes", &home.path(".agents/skills/notes"));
    home.write(
        ".config/opencode/opencode.json",
        r#"{"permission": {"skill": {"*": "deny", "notes": "deny"}}}"#,
    );
    let inv = scan(&home.env(), &only(&["claude-code", "opencode"]), None);
    assert_eq!(state(&inv, "inbox", "opencode"), Some(State::Off));
    assert!(skill(&inv, "inbox").states["opencode"].by_pattern);
    assert!(!skill(&inv, "notes").states["opencode"].by_pattern);
    // Only the skill turned off by name is offered for turning off elsewhere.
    let issue = inv.issues.iter().find(|i| i.id == "off-elsewhere").unwrap();
    assert_eq!(issue.skill_names, vec!["notes".to_string()]);
}

#[test]
fn only_your_own_skills_can_be_removed() {
    let home = Home::new();
    home.skill(".codex/skills/.system/imagegen", "imagegen", "Images");
    home.skill(".claude/skills/synced/acct/notes", "notes", "Notes");
    let env = home.env();
    let inv = scan(&env, &all, None);
    let writer = TestWriter::default();
    for name in ["imagegen", "notes"] {
        let found = skill(&inv, name);
        assert!(!removable(found), "{name}");
        assert!(remove_skill_files(&env, &inv, &found.key, &writer).is_err());
    }
    assert!(writer.trashed.borrow().is_empty());
    assert!(home
        .path(".codex/skills/.system/imagegen/SKILL.md")
        .is_file());
}

#[test]
fn issue_fingerprints_change_only_when_the_problem_does() {
    let home = Home::new();
    let real = home.skill(".agents/skills/pdf", "pdf", "One");
    home.skill(".claude/skills/pdf", "pdf", "Two");
    let _ = real;
    let env = home.env();
    let first = scan(&env, &all, None);
    let again = scan(&env, &all, None);
    let print = |inv: &Inventory| {
        inv.issues
            .iter()
            .find(|i| i.id == "clashes")
            .map(|i| i.fingerprint.clone())
            .unwrap()
    };
    assert!(print(&first).starts_with("clashes:"));
    assert_eq!(print(&first), print(&again));
    home.skill(".claude/skills/docx", "docx", "Three");
    home.skill(".agents/skills/docx", "docx", "Four");
    assert_ne!(print(&first), print(&scan(&env, &all, None)));
}

#[test]
fn folder_issue_fingerprints_follow_the_folders_not_the_wording() {
    let home = Home::new();
    let synced = ".claude/skills/synced";
    home.skill(&format!("{synced}/a/one"), "one", "One");
    home.skill(&format!("{synced}/b/two"), "two", "Two");
    let env = home.env();
    let print = |inv: &Inventory| {
        inv.issues
            .iter()
            .find(|i| i.id == "stale-sync")
            .map(|i| i.fingerprint.clone())
            .unwrap()
    };
    let first = scan(&env, &all, None);
    let stale = first.issues.iter().find(|i| i.id == "stale-sync").unwrap();
    assert_eq!(stale.subject, vec!["~/.claude/skills/synced/a".to_string()]);
    assert_eq!(print(&first), print(&scan(&env, &all, None)));
    home.skill(&format!("{synced}/c/three"), "three", "Three");
    assert_ne!(print(&first), print(&scan(&env, &all, None)));
}

#[test]
fn a_clash_lists_only_the_copies_that_meet_in_one_client() {
    let home = Home::new();
    home.skill(".claude/skills/pdf", "pdf", "Mine");
    home.skill(".claude/skills/synced/team/pdf", "pdf", "Team");
    // Hermes has its own lone pdf; it isn't part of Claude Code's clash, even though Hermes
    // has a clash of its own on another name.
    home.skill(".hermes/skills/productivity/pdf", "pdf", "Hermes");
    home.skill(".hermes/skills/a/notes", "notes", "One");
    home.skill(".hermes/skills/b/notes", "notes", "Two");
    let inv = scan(&home.env(), &only(&["claude-code", "hermes"]), None);
    let clash = inv.issues.iter().find(|i| i.id == "clashes").unwrap();
    assert_eq!(
        clash.agents,
        vec!["claude-code".to_string(), "hermes".to_string()]
    );
    assert_eq!(clash.skills.len(), 4, "{:?}", clash.skills);
    assert!(!clash.skills.iter().any(|k| k.contains("productivity/pdf")));
}

#[test]
fn a_parked_skill_can_be_removed() {
    let home = Home::new();
    let real = home.skill(".agents/disabled-skills/old", "old", "Parked");
    let env = home.env();
    let inv = scan(&env, &all, None);
    let parked = skill(&inv, "old");
    assert!(parked.removable && parked.trash_on_remove);
    let writer = TestWriter::default();

    let (trashed, links) = remove_skill_files(&env, &inv, &parked.key, &writer).unwrap();

    assert_eq!(trashed.as_deref(), real.parent());
    assert_eq!(links, 0);
}

#[test]
fn a_skill_in_place_is_removable_even_when_a_linked_folder_also_reaches_it() {
    let home = Home::new();
    let real = home.skill(".agents/skills/foo", "foo", "Foo");
    // Codex walks its folder deeply, so this link reaches foo a second way.
    home.link(".codex/skills/shared", &home.path(".agents/skills"));
    let env = home.env();
    let inv = scan(&env, &all, None);
    let key = skill(&inv, "foo").key.clone();
    let writer = TestWriter::default();

    let (trashed, links) = remove_skill_files(&env, &inv, &key, &writer).unwrap();

    assert_eq!(trashed.as_deref(), real.parent());
    assert_eq!(links, 0);
    // The linked folder leads to other things too, so it stays.
    assert!(is_link(&home.path(".codex/skills/shared")));
}

#[test]
fn a_fix_waits_for_the_problem_the_user_saw() {
    let home = Home::new();
    home.link(".agents/skills/gone", &home.path("Projects/old/skill"));
    let inv = scan(&home.env(), &all, None);
    let seen = inv
        .issues
        .iter()
        .find(|i| i.id == "broken-links")
        .unwrap()
        .fingerprint
        .clone();
    assert_eq!(still_as_seen(&inv, "broken-links", &seen), Ok(()));
    // A second broken link turned up before the fix ran.
    home.link(
        ".agents/skills/gone-too",
        &home.path("Projects/older/skill"),
    );
    let newer = scan(&home.env(), &all, None);
    let error = still_as_seen(&newer, "broken-links", &seen).unwrap_err();
    assert!(error.contains("changed"), "{error}");
    assert!(still_as_seen(&newer, "clashes", "clashes:0").is_err());
}

#[test]
fn a_link_into_an_unmounted_drive_is_not_offered_for_removal() {
    let home = Home::new();
    home.link(
        ".agents/skills/nas-skill",
        Path::new("/toolport-test-unmounted/skills/nas-skill"),
    );
    home.link(".agents/skills/gone", &home.path("Projects/old/skill"));
    let env = home.env();
    let inv = scan(&env, &all, None);
    assert_eq!(state(&inv, "nas-skill", "codex"), Some(State::Broken));
    let issue = inv.issues.iter().find(|i| i.id == "broken-links").unwrap();
    assert_eq!(issue.skill_names, vec!["gone".to_string()]);
    assert_eq!(
        apply_fix(&env, &inv, "broken-links", &TestWriter::default()),
        Ok(1)
    );
    assert!(is_link(&home.path(".agents/skills/nas-skill")));
    // Open and Reveal on a broken link show the folder it sits in.
    assert!(dangling(&home.path(".agents/skills/nas-skill")));
}

#[test]
fn a_skill_on_a_second_drive_is_yours_not_a_package() {
    assert!(system_path(Path::new("/usr/share/skills/x/SKILL.md")));
    assert!(system_path(Path::new("/opt/tool/skills/x/SKILL.md")));
    assert!(!system_path(Path::new("/data/skills/x/SKILL.md")));
    assert!(!system_path(Path::new("/mnt/work/skills/x/SKILL.md")));
}

#[test]
fn imports_inside_a_sentence_count_but_code_does_not() {
    let base = Path::new("/p");
    let home = Path::new("/h");
    let text = "See @README.md and @docs/style.md, then ask @someone.\n\
                Run `@not/this.md` first.\n\
                ```\n@also/not.md\n```\n\
                @~/rules/a.md\n\
                Also (see @notes.md).\n";
    assert_eq!(
        imports(text, base, home),
        vec![
            PathBuf::from("/p/README.md"),
            PathBuf::from("/p/docs/style.md"),
            PathBuf::from("/p/someone"),
            PathBuf::from("/h/rules/a.md"),
            PathBuf::from("/p/notes.md"),
        ]
    );
}

#[test]
fn opencode_v2_turn_on_counts_a_v1_blanket_deny_beside_it() {
    let home = Home::new();
    home.skill(".agents/skills/x", "x", "X");
    home.write(
        ".config/opencode/opencode.json",
        r#"{"permission": "deny", "permissions": [{"action": "skill", "resource": "x", "effect": "deny"}]}"#,
    );
    let env = home.env();
    let inv = scan(&env, &all, None);
    let key = skill(&inv, "x").key.clone();
    set_state(
        &env,
        &inv,
        "opencode",
        &key,
        Wanted::On,
        &TestWriter::default(),
    )
    .unwrap();
    let after = scan(&env, &all, None);
    assert_eq!(state(&after, "x", "opencode"), Some(State::On));
}

#[test]
fn each_client_walks_its_folders_the_way_it_really_does() {
    let home = Home::new();
    // Claude Code reads one level, plus claude.ai's synced buckets.
    home.skill(".claude/skills/group/nested", "nested", "Nested");
    home.skill(".claude/skills/synced/bucket/account", "account", "Account");
    // Codex goes deep but skips a SKILL.md that is a link.
    home.skill(".codex/skills/group/deep", "deep", "Deep");
    let elsewhere = home.skill("store/filelink", "filelink", "File link");
    std::fs::create_dir_all(home.path(".codex/skills/filelink")).unwrap();
    home.link(".codex/skills/filelink/SKILL.md", &elsewhere);
    // Continue skips a skill folder that is a link.
    let shared = home.skill("store/dirlink", "dirlink", "Dir link");
    home.link(".continue/skills/dirlink", shared.parent().unwrap());
    home.skill(".continue/skills/own", "own", "Own");

    let inv = scan(&home.env(), &all, None);
    assert_eq!(state(&inv, "nested", "claude-code"), None);
    assert_eq!(state(&inv, "account", "claude-code"), Some(State::On));
    assert_eq!(state(&inv, "deep", "codex"), Some(State::On));
    assert_eq!(state(&inv, "filelink", "codex"), None);
    // Cursor reads Codex's folder and follows the link.
    assert_eq!(state(&inv, "filelink", "cursor"), Some(State::On));
    // Nobody else reads that folder, so the linked skill isn't listed at all.
    assert!(!inv.skills.iter().any(|s| s.name == "dirlink"));
    assert_eq!(state(&inv, "own", "continue"), Some(State::On));
}

#[test]
fn rules_files_match_what_each_client_reads() {
    let home = Home::new();
    home.write(".claude/CLAUDE.md", "# Claude\n");
    // OpenCode 2 has no CLAUDE.md fallback.
    home.write(".config/opencode/.keep", "");
    // Pi: the first name in its chain wins.
    home.write(".pi/agent/AGENTS.md", "# Pi\n");
    home.write(".pi/agent/CLAUDE.md", "# skipped\n");
    // Gemini CLI follows context.fileName.
    home.write(
        ".gemini/settings.json",
        r#"{"context": {"fileName": ["AGENTS.md", "GEMINI.md"]}}"#,
    );
    home.write(".gemini/AGENTS.md", "# Gemini\n");
    // Antigravity loads a rules file only with trigger: always_on.
    home.write(
        ".gemini/config/rules/always.md",
        "---\ntrigger: always_on\n---\nYes\n",
    );
    home.write(
        ".gemini/config/rules/manual.md",
        "---\ntrigger: manual\n---\nNo\n",
    );
    // Copilot's instructions folder.
    home.write(".copilot/instructions/style.instructions.md", "Style\n");
    home.write(
        ".copilot/instructions/notes.md",
        "Not an instructions file\n",
    );
    // Grok adds up its own files and the Cursor ones.
    home.write(".grok/rules/extra.md", "Extra\n");
    home.write(".cursor/AGENTS.md", "Cursor\n");

    let inv = scan(&home.env(), &all, None);
    let readers = |path: &str| -> Vec<String> {
        inv.rules
            .iter()
            .find(|r| r.path == path)
            .map(|r| r.agents.keys().cloned().collect())
            .unwrap_or_default()
    };
    assert!(!readers("~/.claude/CLAUDE.md").contains(&"opencode".to_string()));
    assert!(readers("~/.claude/CLAUDE.md").contains(&"grok".to_string()));
    assert_eq!(readers("~/.pi/agent/AGENTS.md"), vec!["pi".to_string()]);
    assert!(readers("~/.pi/agent/CLAUDE.md").is_empty());
    assert!(readers("~/.gemini/AGENTS.md").contains(&"gemini-cli".to_string()));
    assert!(readers("~/.gemini/config/rules/always.md").contains(&"antigravity".to_string()));
    assert!(readers("~/.gemini/config/rules/manual.md").is_empty());
    assert!(readers("~/.copilot/instructions/style.instructions.md")
        .contains(&"github-copilot-cli".to_string()));
    assert!(readers("~/.copilot/instructions/notes.md").is_empty());
    assert!(readers("~/.grok/rules/extra.md").contains(&"grok".to_string()));
    assert!(readers("~/.cursor/AGENTS.md").contains(&"grok".to_string()));
}

#[test]
fn hermes_reads_the_extra_folders_its_config_lists() {
    let home = Home::new();
    home.skill(".agents/skills/shared", "shared", "Shared");
    let env = home.env();
    assert_eq!(state(&scan(&env, &all, None), "shared", "hermes"), None);
    home.write(
        ".hermes/config.yaml",
        "skills:\n  external_dirs:\n    - ~/.agents/skills\n",
    );
    assert_eq!(
        state(&scan(&env, &all, None), "shared", "hermes"),
        Some(State::On)
    );
}

#[test]
fn auggie_command_code_and_tabnine_have_columns() {
    let home = Home::new();
    home.skill(".augment/skills/a", "a", "A");
    home.skill(".commandcode/skills/c", "c", "C");
    home.skill(".tabnine/agent/skills/t", "t", "T");
    let inv = scan(
        &home.env(),
        &only(&["auggie", "command-code", "tabnine"]),
        None,
    );
    assert_eq!(state(&inv, "a", "auggie"), Some(State::On));
    assert_eq!(state(&inv, "c", "command-code"), Some(State::On));
    assert_eq!(state(&inv, "t", "tabnine"), Some(State::On));
}

#[test]
fn grok_edits_keep_the_skills_table_and_refuse_a_list_they_cannot_read() {
    let home = Home::new();
    let writer = TestWriter::default();
    let path = home.write(
        ".grok/config.toml",
        "[skills]\npaths = [\"~/more\"]\ndisabled = [\"old\"]\n",
    );
    edit_grok("grok", &path, &writer, "new", true).unwrap();
    let text = home.read(".grok/config.toml");
    assert!(text.contains("paths = [\"~/more\"]"), "{text}");
    assert!(text.contains("disabled = [\"old\", \"new\"]"), "{text}");
    edit_grok("grok", &path, &writer, "old", false).unwrap();
    assert!(home
        .read(".grok/config.toml")
        .contains("disabled = [\"new\"]"));

    let inline = home.write(".grok/inline.toml", "skills = { paths = [\"~/more\"] }\n");
    edit_grok("grok", &inline, &writer, "x", true).unwrap();
    let text = home.read(".grok/inline.toml");
    assert!(
        text.contains("paths") && text.contains("disabled = [\"x\"]"),
        "{text}"
    );

    let odd = home.write(".grok/odd.toml", "[skills]\ndisabled = \"x\"\n");
    assert!(edit_grok("grok", &odd, &writer, "y", true).is_err());
    assert_eq!(home.read(".grok/odd.toml"), "[skills]\ndisabled = \"x\"\n");
}

#[test]
fn project_folders_are_walked_the_way_each_client_walks_them() {
    let home = Home::new();
    let project = home.path("work/app");
    home.skill("work/app/.claude/skills/group/nested", "nested", "Nested");
    let elsewhere = home.skill("store/linked", "linked", "Linked");
    std::fs::create_dir_all(project.join(".agents/skills/linked")).unwrap();
    home.link("work/app/.agents/skills/linked/SKILL.md", &elsewhere);
    // The same name personally and in the project: Gemini CLI lets the project win.
    home.skill(".gemini/skills/style", "style", "Personal");
    home.skill("work/app/.gemini/skills/style", "style", "Project");
    let inv = scan(&home.env(), &all, Some(&project));

    assert_eq!(state(&inv, "nested", "claude-code"), None);
    assert_eq!(state(&inv, "nested", "cursor"), Some(State::On));
    assert_eq!(state(&inv, "linked", "codex"), None);
    let winner = |agent: &str| {
        inv.skills
            .iter()
            .find(|s| {
                s.name == "style" && s.states.get(agent).is_some_and(|st| st.state == State::On)
            })
            .map(|s| s.description.clone())
    };
    assert_eq!(winner("gemini-cli").as_deref(), Some("Project"));
}

#[test]
fn a_rules_folder_that_links_back_on_itself_is_read_once() {
    let home = Home::new();
    home.write(".continue/rules/style.md", "Style\n");
    home.link(".continue/rules/again", &home.path(".continue/rules"));
    home.link(".continue/rules/home", &home.root);
    let inv = scan(&home.env(), &only(&["continue"]), None);
    let continue_rules: Vec<&str> = inv
        .rules
        .iter()
        .filter(|r| r.agents.contains_key("continue"))
        .map(|r| r.path.as_str())
        .collect();
    assert_eq!(continue_rules, vec!["~/.continue/rules/style.md"]);
}

#[test]
fn imports_resolve_next_to_the_link_or_its_target_per_client() {
    let home = Home::new();
    home.write(".agents/AGENTS.md", "@x.md\n");
    home.write(".agents/x.md", "Next to the target\n");
    home.write(".qwen/x.md", "Next to the link\n");
    home.link(".qwen/QWEN.md", &home.path(".agents/AGENTS.md"));
    home.link(".claude/CLAUDE.md", &home.path(".agents/AGENTS.md"));
    let inv = scan(&home.env(), &only(&["claude-code", "qwen-code"]), None);
    let readers = |path: &str| -> Vec<String> {
        inv.rules
            .iter()
            .find(|r| r.path == path)
            .map(|r| r.agents.keys().cloned().collect())
            .unwrap_or_default()
    };
    assert_eq!(readers("~/.agents/x.md"), vec!["claude-code".to_string()]);
    assert_eq!(readers("~/.qwen/x.md"), vec!["qwen-code".to_string()]);
}

#[test]
fn goose_skips_a_hints_file_that_links_outside_its_folder() {
    let home = Home::new();
    home.write("store/rules.md", "Shared\n");
    home.link(".config/goose/AGENTS.md", &home.path("store/rules.md"));
    home.write(".config/goose/.goosehints", "Own\n");
    let inv = scan(&home.env(), &only(&["goose"]), None);
    assert!(inv
        .rules
        .iter()
        .any(|r| r.path == "~/.config/goose/.goosehints"));
    assert!(!inv
        .rules
        .iter()
        .any(|r| r.agents.contains_key("goose") && r.path.contains("store/rules.md")));
}

#[test]
fn gemini_reads_its_configured_file_names_in_a_project_too() {
    let home = Home::new();
    let project = home.path("work/app");
    home.write(
        ".gemini/settings.json",
        r#"{"context": {"fileName": "AGENTS.md"}}"#,
    );
    home.write("work/app/AGENTS.md", "Project\n");
    home.write("work/app/GEMINI.md", "Not read\n");
    let inv = scan(&home.env(), &only(&["gemini-cli"]), Some(&project));
    let gemini = |name: &str| {
        inv.rules
            .iter()
            .any(|r| r.name == name && r.agents.contains_key("gemini-cli"))
    };
    assert!(gemini("AGENTS.md"));
    assert!(!gemini("GEMINI.md"));
}

#[test]
fn clients_without_a_rules_file_yet_still_offer_where_one_goes() {
    let home = Home::new();
    // Another client has rules, so the ones without stand out.
    home.write(".codex/AGENTS.md", "# Codex\n");
    let inv = scan(
        &home.env(),
        &only(&["codex", "grok", "pi", "antigravity"]),
        None,
    );
    let locations = |id: &str| {
        inv.agents
            .iter()
            .find(|a| a.id == id)
            .map(|a| a.rule_locations.clone())
            .unwrap_or_default()
    };
    assert!(locations("grok").contains(&"~/.grok/AGENTS.md".to_string()));
    assert!(locations("pi").contains(&"~/.pi/agent/AGENTS.md".to_string()));
    assert!(locations("antigravity").contains(&"~/.gemini/config/AGENTS.md".to_string()));
    let missing = inv.issues.iter().find(|i| i.id == "rules-missing").unwrap();
    assert!(missing.agents.contains(&"grok".to_string()));
    assert!(missing.agents.contains(&"pi".to_string()));
}
