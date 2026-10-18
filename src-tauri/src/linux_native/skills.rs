//! Skills and rules page: what each installed agent loads besides MCP tools.
//! The scan and every write live in `crate::skills`; this page only draws it, with the same
//! layout as the React view: a list with one status column per agent, and a detail pane.

use super::{branding, i18n as l10n};
use crate::skills::{Agent, Inventory, Issue, RuleFile, Skill, Source, State, Wanted};
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::Rc;

use adw::prelude::*;

const SOURCE_ORDER: [Source; 7] = [
    Source::Yours,
    Source::Account,
    Source::Package,
    Source::BuiltIn,
    Source::Library,
    Source::Parked,
    Source::Broken,
];
const ISSUE_SKILL_LIMIT: usize = 12;
/// The collapsed-group key for dismissed Needs a look items.
const DISMISSED_KEY: &str = "issues:dismissed";
/// Width of one agent's status column, shared by the header logos and the row dots.
const COLUMN: i32 = 28;
const SIZE_COLUMN: i32 = 64;
const DETAIL_WIDTH: i32 = 380;
/// Below this page width the detail pane moves under the list, as the React view does.
const NARROW: &str = "max-width: 980px";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Tab {
    Skills,
    Rules,
    Issues,
}

impl Tab {
    fn name(self) -> &'static str {
        match self {
            Tab::Skills => "skills",
            Tab::Rules => "rules",
            Tab::Issues => "issues",
        }
    }
}

/// What a row in one of the two lists stands for, by position.
#[derive(Clone)]
enum RowKind {
    Group(String),
    Item(String),
}

/// One list-and-detail tab (Skills or Rules).
#[derive(Clone)]
struct Pane {
    banner: gtk::Box,
    head: gtk::Box,
    list: gtk::ListBox,
    scroll: gtk::ScrolledWindow,
    detail: gtk::Box,
    detail_scroll: gtk::ScrolledWindow,
    rows: Rc<RefCell<Vec<RowKind>>>,
    layout: gtk::Box,
    separator: gtk::Separator,
    /// Source strings for the two panes' accessible names, read again on a language change.
    names: (&'static str, &'static str),
}

/// A write to run off the main thread, bound to the project that was open when it was asked
/// for. It returns the fresh scan and the message to show.
type Write = Box<dyn FnOnce() -> Result<(Inventory, String), String> + Send>;

#[derive(Clone)]
pub(super) struct SkillsPage {
    pub(super) root: gtk::Box,
    /// `loading`, `error`, `empty` or `content`.
    screens: gtk::Stack,
    failed_body: gtk::Label,
    app: adw::Application,
    stack: gtk::Stack,
    tabs: Rc<Vec<(Tab, gtk::ToggleButton, gtk::Label)>>,
    skills: Pane,
    rules: Pane,
    issues_list: gtk::Box,
    issues_side: gtk::Box,
    search: gtk::SearchEntry,
    feedback: gtk::Label,
    /// Hides a success message after a few seconds; errors stay until the next action.
    feedback_timer: Rc<RefCell<Option<gtk::glib::SourceId>>>,
    project_label: gtk::Label,
    project_path: gtk::Label,
    refresh_button: gtk::Button,
    inventory: Rc<RefCell<Option<Inventory>>>,
    project: Rc<RefCell<Option<String>>>,
    generation: Rc<Cell<u64>>,
    busy: Rc<Cell<bool>>,
    /// Writes asked for while one runs. They follow in order, so a quick second click is kept
    /// and no control has to be greyed out (which would take keyboard focus away).
    queued: Rc<RefCell<VecDeque<Write>>>,
    /// The first error from a run of queued writes, shown once the last one finishes.
    queued_error: Rc<RefCell<Option<String>>>,
    /// A refresh asked for while a write runs; it starts when the write ends so it never reads
    /// the file halfway through a change.
    refresh_pending: Rc<Cell<bool>>,
    tab: Rc<Cell<Tab>>,
    /// The agent whose column was clicked: the list shows only what it loads.
    focus: Rc<RefCell<Option<String>>>,
    selected_skill: Rc<RefCell<Option<String>>>,
    selected_rule: Rc<RefCell<Option<String>>>,
    collapsed: Rc<RefCell<HashSet<String>>>,
    /// Set while a list is rebuilt, so selecting its first row doesn't count as a click.
    rebuilding: Rc<Cell<bool>>,
    /// The detail pane's state controls by agent, to give focus back after a write redraws them.
    controls: Rc<RefCell<HashMap<String, gtk::Widget>>>,
    last_control: Rc<RefCell<Option<String>>>,
    /// The On / Name only / Off button that last had keyboard focus (`agent#value`), so a
    /// redraw puts focus back where the arrow keys left it, not on the saved state.
    focused_choice: Rc<RefCell<Option<String>>>,
    /// The control a click is about to redraw (a group header, a column filter), so the redrawn
    /// copy gets keyboard focus back.
    refocus: Rc<RefCell<Option<String>>>,
    refocus_widget: Rc<RefCell<Option<gtk::Widget>>>,
}

/// The folder a skill's file sits in, as shown. The file may be a link to one with another
/// name, so this drops the last part of the path whatever it is called. A broken link has no
/// file; the link is the folder.
fn own_folder(skill: &Skill) -> &str {
    if skill.source == Source::Broken {
        return &skill.path;
    }
    skill
        .path
        .rfind(['/', '\\'])
        .map_or(skill.path.as_str(), |at| &skill.path[..at])
}

fn muted(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Fill)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(["toolport-muted"])
        .build()
}

fn one_line(text: &str, classes: &[&str]) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Fill)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .css_classes(classes.to_vec())
        .build()
}

fn heading(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .css_classes(["heading"])
        .build()
}

fn section(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Start)
        .hexpand(true)
        .xalign(0.0)
        .css_classes(["toolport-section-label"])
        .build()
}

/// A section label with a muted note on the right, like "AGENTS … 7 of 7 load it".
fn section_with(text: &str, aside: &str) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    row.set_margin_top(6);
    row.append(&section(text));
    let note = muted(aside);
    note.set_wrap(false);
    note.set_hexpand(false);
    note.add_css_class("caption");
    row.append(&note);
    row
}

fn badge(text: &str, class: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .valign(gtk::Align::Center)
        .css_classes(["toolport-badge", "caption"])
        .build();
    if !class.is_empty() {
        label.add_css_class(class);
    }
    label
}

fn icon_badge(icon: &str, text: &str, class: &str) -> gtk::Box {
    let chip = gtk::Box::new(gtk::Orientation::Horizontal, 5);
    chip.set_valign(gtk::Align::Center);
    chip.add_css_class("toolport-badge");
    chip.add_css_class("caption");
    if !class.is_empty() {
        chip.add_css_class(class);
    }
    let image = gtk::Image::from_icon_name(icon);
    image.set_pixel_size(12);
    chip.append(&image);
    chip.append(&gtk::Label::new(Some(text)));
    chip
}

fn logo(id: &str, size: i32) -> gtk::Image {
    let image = branding::client_logo(id);
    image.set_pixel_size(size);
    image
}

/// An agent's logo on a small rounded square, as in the detail and By agent lists.
fn logo_tile(id: &str) -> gtk::CenterBox {
    let tile = gtk::CenterBox::new();
    tile.add_css_class("toolport-skill-logo");
    tile.set_size_request(32, 32);
    tile.set_hexpand(false);
    tile.set_valign(gtk::Align::Start);
    tile.set_center_widget(Some(&logo(id, 20)));
    tile
}

/// A rounded square holding an icon, the way the mockup heads details and issues.
fn tile(icon: &str, class: &str, size: i32) -> gtk::Box {
    let tile = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    tile.add_css_class("toolport-skill-tile");
    if !class.is_empty() {
        tile.add_css_class(class);
    }
    tile.set_size_request(size, size);
    tile.set_valign(gtk::Align::Start);
    tile.set_halign(gtk::Align::Start);
    // Explicit: a child's expand flag would otherwise stretch the tile across its row.
    tile.set_hexpand(false);
    tile.set_vexpand(false);
    let image = gtk::Image::from_icon_name(icon);
    image.set_halign(gtk::Align::Center);
    image.set_valign(gtk::Align::Center);
    image.set_hexpand(true);
    image.set_pixel_size(size / 2);
    tile.append(&image);
    tile
}

/// The first part of a file, as the React view's Preview block shows it.
fn preview_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Fill)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .selectable(true)
        .css_classes(["monospace", "toolport-muted", "toolport-value-card"])
        .build()
}

fn clear_box(list: &gtk::Box) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

fn clear_list(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }
}

/// Decimal units, matching the React view's `fmtBytes`.
fn fmt_bytes(bytes: u64) -> String {
    if bytes >= 999_950 {
        format!("{:.1} MB", bytes as f64 / 1_000_000.0)
    } else if bytes >= 1_000 {
        format!("{:.1} KB", bytes as f64 / 1_000.0)
    } else {
        format!("{bytes} B")
    }
}

fn fmt_date(secs: u64) -> Option<String> {
    let date = gtk::glib::DateTime::from_unix_local(i64::try_from(secs).ok()?).ok()?;
    date.format("%x").ok().map(|text| text.to_string())
}

fn folder_name(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string())
}

fn source_key(source: Source) -> String {
    format!("{source:?}")
}

fn source_label(source: Source) -> String {
    crate::i18n::lookup(match source {
        Source::Yours => crate::i18n::msg("Yours"),
        Source::Account => crate::i18n::msg("From your claude.ai account"),
        Source::Package => crate::i18n::msg("System package"),
        Source::BuiltIn => crate::i18n::msg("Built into a client"),
        Source::Library => crate::i18n::msg("Client library"),
        Source::Parked => crate::i18n::msg("Parked"),
        Source::Broken => crate::i18n::msg("Broken links"),
    })
}

fn source_icon(source: Source) -> &'static str {
    match source {
        Source::Yours => "avatar-default-symbolic",
        Source::Account => "weather-overcast-symbolic",
        Source::Package => "package-x-generic-symbolic",
        Source::BuiltIn => "changes-prevent-symbolic",
        Source::Library => "folder-symbolic",
        Source::Parked => "action-unavailable-symbolic",
        Source::Broken => "dialog-warning-symbolic",
    }
}

fn state_label(state: Option<State>) -> String {
    crate::i18n::lookup(match state {
        Some(State::On) => crate::i18n::msg("On"),
        Some(State::NameOnly) => crate::i18n::msg("Name only"),
        Some(State::Off) => crate::i18n::msg("Off"),
        Some(State::Shadowed) => crate::i18n::msg("Another copy wins"),
        Some(State::Broken) => crate::i18n::msg("Broken"),
        None => crate::i18n::msg("Not loaded"),
    })
}

/// One cell of the agent matrix. Shape and color both carry the state, as in the React view.
fn state_dot(state: Option<State>, tooltip: &str) -> gtk::CenterBox {
    // Named like the React view's role="img" dots; the legend's dots sit beside their own label.
    let cell = gtk::CenterBox::builder()
        .accessible_role(if tooltip.is_empty() {
            gtk::AccessibleRole::Presentation
        } else {
            gtk::AccessibleRole::Img
        })
        .build();
    cell.set_size_request(COLUMN, 20);
    cell.set_valign(gtk::Align::Center);
    cell.set_hexpand(false);
    if !tooltip.is_empty() {
        cell.set_tooltip_text(Some(tooltip));
        cell.update_property(&[gtk::accessible::Property::Label(tooltip)]);
    }
    let shape: gtk::Widget = if state == Some(State::Off) {
        let image = gtk::Image::from_icon_name("action-unavailable-symbolic");
        image.set_pixel_size(14);
        image.add_css_class("toolport-skill-off");
        image.upcast()
    } else {
        let dot = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        dot.add_css_class("toolport-skill-dot");
        dot.add_css_class(match state {
            Some(State::On) => "on",
            Some(State::NameOnly) => "name-only",
            Some(State::Shadowed) => "shadowed",
            Some(State::Broken) => "broken",
            _ => "none",
        });
        if state == Some(State::Broken) {
            let inner = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            inner.add_css_class("toolport-skill-dot-core");
            inner.set_halign(gtk::Align::Center);
            inner.set_valign(gtk::Align::Center);
            dot.set_halign(gtk::Align::Center);
            let holder = gtk::CenterBox::new();
            holder.set_center_widget(Some(&inner));
            dot.append(&holder);
        }
        dot.upcast()
    };
    shape.set_halign(gtk::Align::Center);
    shape.set_valign(gtk::Align::Center);
    cell.set_center_widget(Some(&shape));
    cell
}

fn legend() -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    row.add_css_class("toolport-skill-legend");
    for (state, label) in [
        (Some(State::On), l10n::label("On")),
        (Some(State::NameOnly), l10n::label("Name only")),
        (Some(State::Off), l10n::label("Off")),
        (Some(State::Shadowed), l10n::label("Another copy wins")),
        (Some(State::Broken), l10n::label("Broken")),
        (None, l10n::label("Not loaded")),
    ] {
        let item = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        item.append(&state_dot(state, ""));
        label.add_css_class("caption");
        label.add_css_class("toolport-muted");
        item.append(&label);
        row.append(&item);
    }
    row
}

fn skill_agents(inv: &Inventory) -> Vec<&Agent> {
    inv.agents.iter().filter(|a| a.supports_skills).collect()
}

fn project_state_path() -> Option<std::path::PathBuf> {
    Some(crate::registry::conduit_dir()?.join("gtk-skills-project.json"))
}

/// The project folder picked last time, like the React view's saved choice.
fn load_project() -> Option<String> {
    let text = std::fs::read_to_string(project_state_path()?).ok()?;
    let value: serde_json::Value = serde_json::from_str(&text).ok()?;
    value
        .get("project")
        .and_then(|p| p.as_str())
        .map(str::to_string)
}

fn save_project(project: Option<&str>) {
    let Some(path) = project_state_path() else {
        return;
    };
    match project {
        Some(project) => {
            let _ = std::fs::write(path, serde_json::json!({ "project": project }).to_string());
        }
        None => {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn stack_when_narrow(
    narrow: &adw::Breakpoint,
    layout: &gtk::Box,
    separator: &gtk::Separator,
    side: &gtk::ScrolledWindow,
) {
    narrow.add_setter(
        layout,
        "orientation",
        Some(&gtk::Orientation::Vertical.to_value()),
    );
    narrow.add_setter(
        separator,
        "orientation",
        Some(&gtk::Orientation::Horizontal.to_value()),
    );
    narrow.add_setter(side, "width-request", Some(&(-1).to_value()));
}

impl Pane {
    /// In a narrow window, stack the detail pane under the list instead of beside it.
    fn stack_when_narrow(&self, narrow: &adw::Breakpoint) {
        stack_when_narrow(narrow, &self.layout, &self.separator, &self.detail_scroll);
    }

    /// The list column (banner, column header, rows, legend) beside a fixed-width detail pane.
    fn new(list_name: &'static str, detail_name: &'static str) -> (Self, gtk::Box) {
        let layout = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let column = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.set_hexpand(true);
        column.set_margin_start(20);
        column.set_margin_end(16);
        column.set_margin_top(14);
        column.set_margin_bottom(12);
        let banner = gtk::Box::new(gtk::Orientation::Vertical, 0);
        column.append(&banner);
        let head = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        head.add_css_class("toolport-skill-head");
        column.append(&head);
        let list = gtk::ListBox::builder()
            .selection_mode(gtk::SelectionMode::Single)
            .css_classes(["toolport-skill-list"])
            .build();
        list.update_property(&[gtk::accessible::Property::Label(&crate::i18n::lookup(
            list_name,
        ))]);
        let scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .vexpand(true)
            .child(&list)
            .build();
        column.append(&scroll);
        column.append(&legend());
        layout.append(&column);
        let separator = gtk::Separator::new(gtk::Orientation::Vertical);
        layout.append(&separator);
        let detail = gtk::Box::new(gtk::Orientation::Vertical, 12);
        detail.set_margin_start(20);
        detail.set_margin_end(20);
        detail.set_margin_top(18);
        detail.set_margin_bottom(20);
        detail.update_property(&[gtk::accessible::Property::Label(&crate::i18n::lookup(
            detail_name,
        ))]);
        let detail_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(DETAIL_WIDTH)
            .vexpand(true)
            .child(&detail)
            .build();
        layout.append(&detail_scroll);
        (
            Self {
                banner,
                head,
                list,
                scroll,
                detail,
                detail_scroll,
                rows: Rc::default(),
                layout: layout.clone(),
                separator,
                names: (list_name, detail_name),
            },
            layout,
        )
    }
}

impl SkillsPage {
    pub(super) fn new(app: &adw::Application) -> Self {
        let content = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let header = adw::HeaderBar::new();
        header.add_css_class("toolport-header");
        header.set_show_back_button(true);
        header.set_title_widget(Some(&{
            let widget = gtk::Label::builder()
                .label(crate::i18n::tr("Skills and rules"))
                .css_classes(["title"])
                .build();
            l10n::bind_label(&widget, "Skills and rules");
            widget
        }));
        content.append(&header);

        let top = gtk::Box::new(gtk::Orientation::Vertical, 10);
        top.set_margin_top(14);
        top.set_margin_start(20);
        top.set_margin_end(20);
        top.append(&{
            let widget = muted(&crate::i18n::tr(
                "What each agent loads besides tools, and where it lives",
            ));
            l10n::bind_label(
                &widget,
                "What each agent loads besides tools, and where it lives",
            );
            widget
        });

        // Tabs on the left, project/search/refresh on the right, over one bottom rule.
        let bar = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        bar.add_css_class("toolport-skill-tabbar");
        // Keep the bar's height when Needs a look hides the search box.
        bar.set_height_request(46);
        let tab_box = gtk::Box::new(gtk::Orientation::Horizontal, 18);
        tab_box.set_hexpand(true);
        tab_box.set_valign(gtk::Align::End);
        let mut tabs = Vec::new();
        let mut group: Option<gtk::ToggleButton> = None;
        for (tab, title) in [
            (Tab::Skills, l10n::label("Skills")),
            (Tab::Rules, l10n::label("Rules")),
            (Tab::Issues, l10n::label("Needs a look")),
        ] {
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 7);
            content.append(&title);
            let count = gtk::Label::builder()
                .css_classes(["toolport-skill-count", "caption"])
                .build();
            content.append(&count);
            let button = gtk::ToggleButton::builder()
                .child(&content)
                .css_classes(["flat", "toolport-skill-tab"])
                .build();
            if let Some(first) = &group {
                button.set_group(Some(first));
            } else {
                group = Some(button.clone());
            }
            tab_box.append(&button);
            tabs.push((tab, button, count));
        }
        bar.append(&tab_box);

        // Room for "Global only" before a long project name is shortened in the middle.
        let project_label = gtk::Label::builder()
            .ellipsize(gtk::pango::EllipsizeMode::Middle)
            .width_chars(11)
            .max_width_chars(28)
            .build();
        let project_content = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        project_content.append(&gtk::Image::from_icon_name("folder-open-symbolic"));
        project_content.append(&project_label);
        let popover_box = gtk::Box::new(gtk::Orientation::Vertical, 2);
        popover_box.set_margin_top(4);
        popover_box.set_margin_bottom(4);
        let global_item = l10n::button("Global only");
        global_item.add_css_class("flat");
        let add_item = l10n::button("Add a project folder…");
        add_item.add_css_class("flat");
        for item in [&global_item, &add_item] {
            if let Some(label) = item.child().and_downcast::<gtk::Label>() {
                label.set_xalign(0.0);
            }
            popover_box.append(item);
        }
        let project_path = gtk::Label::builder()
            .xalign(0.0)
            .wrap(true)
            .wrap_mode(gtk::pango::WrapMode::Char)
            .max_width_chars(36)
            .css_classes(["monospace", "caption", "toolport-muted"])
            .margin_start(10)
            .margin_end(10)
            .margin_top(6)
            .build();
        popover_box.append(&project_path);
        let popover = gtk::Popover::builder().child(&popover_box).build();
        let project_menu = gtk::MenuButton::builder()
            .child(&project_content)
            .popover(&popover)
            .always_show_arrow(true)
            .valign(gtk::Align::Center)
            .build();
        project_menu.set_tooltip_text(Some(&crate::i18n::tr(
            "Also show the rules and skills a project folder adds",
        )));
        l10n::bind_tooltip(
            &project_menu,
            "Also show the rules and skills a project folder adds",
        );
        bar.append(&project_menu);
        let search = gtk::SearchEntry::builder()
            .placeholder_text(crate::i18n::tr("Search skills"))
            .width_request(220)
            .valign(gtk::Align::Center)
            .css_classes(["toolport-search"])
            .build();
        bar.append(&search);
        let refresh_button = gtk::Button::from_icon_name("view-refresh-symbolic");
        refresh_button.set_valign(gtk::Align::Center);
        refresh_button.set_tooltip_text(Some(&crate::i18n::tr("Scan again")));
        l10n::bind_tooltip(&refresh_button, "Scan again");
        bar.append(&refresh_button);
        top.append(&bar);

        let feedback = gtk::Label::builder()
            .halign(gtk::Align::Fill)
            .xalign(0.0)
            .wrap(true)
            .css_classes(["toolport-feedback", "idle"])
            .accessible_role(gtk::AccessibleRole::Status)
            .build();
        // Always in place, empty when there is nothing to say, so the rows below never move
        // under the pointer when a message comes or goes.
        top.append(&feedback);
        content.append(&top);

        // Size to the visible tab only, so a wide tab can't push the window past its tile.
        let stack = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .transition_duration(120)
            .hhomogeneous(false)
            .vexpand(true)
            .build();
        use crate::i18n::msg;
        let (skills, skills_layout) = Pane::new(msg("Skills"), msg("Skill details"));
        let (rules, rules_layout) = Pane::new(msg("Rules"), msg("Rules file details"));
        stack.add_named(&skills_layout, Some("skills"));
        stack.add_named(&rules_layout, Some("rules"));

        let issues_layout = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let issues_list = gtk::Box::new(gtk::Orientation::Vertical, 10);
        issues_list.set_margin_start(20);
        issues_list.set_margin_end(16);
        issues_list.set_margin_top(14);
        issues_list.set_margin_bottom(20);
        issues_layout.append(
            &gtk::ScrolledWindow::builder()
                .hscrollbar_policy(gtk::PolicyType::Never)
                .hexpand(true)
                .vexpand(true)
                .child(&issues_list)
                .build(),
        );
        let issues_separator = gtk::Separator::new(gtk::Orientation::Vertical);
        issues_layout.append(&issues_separator);
        let issues_side = gtk::Box::new(gtk::Orientation::Vertical, 10);
        issues_side.set_margin_start(20);
        issues_side.set_margin_end(20);
        issues_side.set_margin_top(18);
        issues_side.set_margin_bottom(20);
        let issues_side_scroll = gtk::ScrolledWindow::builder()
            .hscrollbar_policy(gtk::PolicyType::Never)
            .width_request(DETAIL_WIDTH)
            .vexpand(true)
            .child(&issues_side)
            .build();
        issues_layout.append(&issues_side_scroll);
        stack.add_named(&issues_layout, Some("issues"));
        content.append(&stack);

        // A tiled or half-width window gets the stacked layout instead of a clipped one.
        let narrow = adw::Breakpoint::new(
            adw::BreakpointCondition::parse(NARROW)
                .expect("the Skills page breakpoint is a static valid condition"),
        );
        skills.stack_when_narrow(&narrow);
        rules.stack_when_narrow(&narrow);
        stack_when_narrow(
            &narrow,
            &issues_layout,
            &issues_separator,
            &issues_side_scroll,
        );
        narrow.add_setter(&search, "width-request", Some(&150.to_value()));
        let adaptive = adw::BreakpointBin::builder()
            .child(&content)
            .width_request(360)
            .height_request(300)
            .hexpand(true)
            .vexpand(true)
            .build();
        adaptive.add_breakpoint(narrow);
        // The first scan, a scan that failed with nothing to show, and no agents at all each get
        // a whole-page screen, as in the React view.
        let screens = gtk::Stack::builder()
            .transition_type(gtk::StackTransitionType::Crossfade)
            .vexpand(true)
            .build();
        let loading = gtk::Box::new(gtk::Orientation::Vertical, 0);
        loading.set_valign(gtk::Align::Center);
        let spinner = gtk::Spinner::builder().spinning(true).build();
        spinner.set_size_request(32, 32);
        spinner.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr(
            "Loading…",
        ))]);
        loading.append(&spinner);
        screens.add_named(&loading, Some("loading"));
        let failed = super::state_card(
            "dialog-warning-symbolic",
            "Couldn't read skills and rules",
            "",
            true,
        );
        failed.set_valign(gtk::Align::Center);
        let failed_body = failed
            .last_child()
            .and_downcast::<gtk::Label>()
            .expect("state card body label");
        let try_again = l10n::button("Try again");
        try_again.set_halign(gtk::Align::Center);
        failed.append(&try_again);
        screens.add_named(&failed, Some("error"));
        let empty = super::state_card(
            "system-search-symbolic",
            "No agents found",
            "Install an AI coding agent such as Claude Code, Codex or Cursor, then scan again.",
            false,
        );
        empty.set_valign(gtk::Align::Center);
        let scan_again = l10n::button("Scan again");
        scan_again.set_halign(gtk::Align::Center);
        empty.append(&scan_again);
        screens.add_named(&empty, Some("empty"));
        screens.add_named(&adaptive, Some("content"));
        screens.set_visible_child_name("loading");
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.add_css_class("toolport-content");
        root.append(&screens);

        let page = Self {
            root,
            screens,
            failed_body,
            app: app.clone(),
            stack,
            tabs: Rc::new(tabs),
            skills,
            rules,
            issues_list,
            issues_side,
            search,
            feedback,
            feedback_timer: Rc::default(),
            project_label,
            project_path,
            refresh_button,
            inventory: Rc::default(),
            project: Rc::new(RefCell::new(load_project())),
            generation: Rc::default(),
            busy: Rc::default(),
            refocus: Rc::default(),
            refocus_widget: Rc::default(),
            queued: Rc::default(),
            queued_error: Rc::default(),
            refresh_pending: Rc::default(),
            tab: Rc::new(Cell::new(Tab::Skills)),
            focus: Rc::default(),
            selected_skill: Rc::default(),
            selected_rule: Rc::default(),
            // Open on your own skills; the rest are a click away.
            collapsed: Rc::new(RefCell::new(
                [
                    Source::Account,
                    Source::Package,
                    Source::BuiltIn,
                    Source::Library,
                    Source::Parked,
                ]
                .into_iter()
                .map(source_key)
                .chain([DISMISSED_KEY.to_string()])
                .collect(),
            )),
            rebuilding: Rc::default(),
            controls: Rc::default(),
            last_control: Rc::default(),
            focused_choice: Rc::default(),
        };
        for (tab, button, _) in page.tabs.iter() {
            let page_ref = page.clone();
            let tab = *tab;
            button.connect_toggled(move |button| {
                if button.is_active() && page_ref.tab.get() != tab {
                    page_ref.select_tab(tab);
                }
            });
        }
        if let Some((_, first, _)) = page.tabs.first() {
            first.set_active(true);
        }
        page.connect_list(&page.skills, true);
        page.connect_list(&page.rules, false);
        {
            let page_ref = page.clone();
            page.refresh_button
                .connect_clicked(move |_| page_ref.refresh());
        }
        {
            let page_ref = page.clone();
            page.search
                .connect_search_changed(move |_| page_ref.render());
        }
        {
            let page_ref = page.clone();
            let popover = popover.clone();
            add_item.connect_clicked(move |_| {
                popover.popdown();
                page_ref.choose_project();
            });
        }
        {
            let page_ref = page.clone();
            global_item.connect_clicked(move |_| {
                popover.popdown();
                page_ref.set_project(None);
            });
        }
        {
            // Scan whenever the page is shown so it never draws stale state.
            let page_ref = page.clone();
            page.root.connect_map(move |_| page_ref.refresh());
        }
        {
            let page_ref = page.clone();
            l10n::on_language_change(&page.root, move || page_ref.retranslate());
        }
        for button in [&try_again, &scan_again] {
            let page_ref = page.clone();
            button.connect_clicked(move |_| page_ref.refresh());
        }
        page.render_project();
        page
    }

    /// Text set from code rather than bound to one source string, after a language change.
    /// Issue titles, notes and states come from the scan, so a visible page scans again.
    fn retranslate(&self) {
        for pane in [&self.skills, &self.rules] {
            pane.list
                .update_property(&[gtk::accessible::Property::Label(&crate::i18n::lookup(
                    pane.names.0,
                ))]);
            pane.detail
                .update_property(&[gtk::accessible::Property::Label(&crate::i18n::lookup(
                    pane.names.1,
                ))]);
        }
        self.set_search_placeholder();
        self.render_project();
        if self.root.is_mapped() {
            self.refresh();
        }
    }

    /// Ctrl+F: the search box, on the Skills tab when Needs a look (which has none) is open.
    pub(super) fn focus_search(&self) {
        if self.tab.get() == Tab::Issues {
            self.select_tab(Tab::Skills);
        }
        self.search.grab_focus();
    }

    fn set_search_placeholder(&self) {
        self.search
            .set_placeholder_text(Some(&if self.tab.get() == Tab::Rules {
                crate::i18n::tr("Search rules")
            } else {
                crate::i18n::tr("Search skills")
            }));
    }

    fn connect_list(&self, pane: &Pane, skills: bool) {
        {
            let page = self.clone();
            let rows = pane.rows.clone();
            pane.list.connect_row_activated(move |_, row| {
                let kind = usize::try_from(row.index())
                    .ok()
                    .and_then(|i| rows.borrow().get(i).cloned());
                if let Some(RowKind::Group(key)) = kind {
                    {
                        let mut collapsed = page.collapsed.borrow_mut();
                        if !collapsed.remove(&key) {
                            collapsed.insert(key.clone());
                        }
                    }
                    page.render_keeping_focus(format!("group:{key}"));
                }
            });
        }
        let page = self.clone();
        let rows = pane.rows.clone();
        let detail_scroll = pane.detail_scroll.clone();
        pane.list.connect_row_selected(move |_, row| {
            if page.rebuilding.get() {
                return;
            }
            let kind = row
                .and_then(|row| usize::try_from(row.index()).ok())
                .and_then(|i| rows.borrow().get(i).cloned());
            if let Some(RowKind::Item(key)) = kind {
                // A click in the list keeps focus there, whatever a write left to restore.
                page.last_control.replace(None);
                page.focused_choice.replace(None);
                let selected = if skills {
                    &page.selected_skill
                } else {
                    &page.selected_rule
                };
                // Another item opens at its top; a redraw of the same one keeps its place.
                if selected.replace(Some(key.clone())).as_ref() != Some(&key) {
                    detail_scroll.vadjustment().set_value(0.0);
                }
                page.render_details();
            }
        });
    }

    fn select_tab(&self, tab: Tab) {
        // An agent filter from one tab can name an agent the other tab has no column for.
        self.focus.replace(None);
        self.last_control.replace(None);
        self.focused_choice.replace(None);
        self.tab.set(tab);
        self.stack.set_visible_child_name(tab.name());
        for (each, button, _) in self.tabs.iter() {
            if *each == tab && !button.is_active() {
                button.set_active(true);
            }
        }
        self.search.set_visible(tab != Tab::Issues);
        self.set_search_placeholder();
        self.render();
    }

    fn set_project(&self, project: Option<String>) {
        if *self.project.borrow() == project {
            return;
        }
        save_project(project.as_deref());
        self.project.replace(project);
        self.render_project();
        self.refresh();
    }

    fn render_project(&self) {
        let project = self.project.borrow().clone();
        self.project_label.set_label(&match &project {
            Some(path) => crate::i18n::trf("Global + {folder}", &[("folder", &folder_name(path))]),
            None => crate::i18n::tr("Global only"),
        });
        self.project_path
            .set_label(project.as_deref().unwrap_or_default());
        self.project_path.set_visible(project.is_some());
    }

    pub(super) fn refresh(&self) {
        if self.busy.get() {
            self.refresh_pending.set(true);
            return;
        }
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        self.refresh_button.set_sensitive(false);
        let project = self.project.borrow().clone();
        let page = self.clone();
        gtk::glib::spawn_future_local(async move {
            let asked = project.clone();
            let result =
                gtk::gio::spawn_blocking(move || crate::skills::inventory(project.as_deref()))
                    .await;
            if generation != page.generation.get() {
                return;
            }
            page.refresh_button.set_sensitive(true);
            let error = match result {
                Ok(Ok(inv)) => {
                    // A fresh scan supersedes an old error or note.
                    page.show_feedback("", false);
                    page.accept(inv, asked.is_some());
                    return;
                }
                Ok(Err(error)) => error,
                Err(_) => crate::i18n::tr("Couldn't read skills and rules"),
            };
            if page.inventory.borrow().is_some() {
                // Keep what is on screen and say the rescan failed.
                page.show_feedback(&error, true);
            } else {
                page.failed_body.set_text(&error);
                page.screens.set_visible_child_name("error");
            }
        });
    }

    /// Take a scan, following the backend when it dropped a project folder that is gone.
    fn accept(&self, inv: Inventory, asked_project: bool) {
        if asked_project && inv.project.is_none() {
            save_project(None);
            self.project.replace(None);
            self.render_project();
            self.show_feedback(
                &crate::i18n::tr("That project folder is gone. Showing global only."),
                false,
            );
        }
        self.inventory.replace(Some(inv));
        self.render();
    }

    /// Scan again without clearing the message a failed write just showed.
    fn rescan_keeping_feedback(&self) {
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let project = self.project.borrow().clone();
        let asked = project.is_some();
        let page = self.clone();
        gtk::glib::spawn_future_local(async move {
            let result =
                gtk::gio::spawn_blocking(move || crate::skills::inventory(project.as_deref()))
                    .await;
            if generation != page.generation.get() {
                return;
            }
            match result {
                // Also notices a project folder that went away during the write.
                Ok(Ok(inv)) => page.accept(inv, asked),
                _ => page.render(),
            }
        });
    }

    fn show_feedback(&self, message: &str, error: bool) {
        if let Some(timer) = self.feedback_timer.borrow_mut().take() {
            timer.remove();
        }
        self.feedback.set_label(message);
        if message.is_empty() {
            self.feedback.add_css_class("idle");
        } else {
            self.feedback.remove_css_class("idle");
        }
        if error {
            self.feedback.add_css_class("error");
        } else {
            self.feedback.remove_css_class("error");
        }
        if !error && !message.is_empty() {
            let feedback = self.feedback.clone();
            let feedback_timer = self.feedback_timer.clone();
            let timer = gtk::glib::timeout_add_seconds_local_once(5, move || {
                feedback_timer.borrow_mut().take();
                feedback.set_label("");
                feedback.add_css_class("idle");
            });
            *self.feedback_timer.borrow_mut() = Some(timer);
        }
    }

    fn choose_project(&self) {
        let Some(parent) = self.app.active_window() else {
            return;
        };
        let dialog = gtk::FileDialog::builder()
            .title(crate::i18n::tr("Choose a project folder"))
            .modal(true)
            .build();
        if let Some(home) = dirs::home_dir() {
            dialog.set_initial_folder(Some(&gtk::gio::File::for_path(home)));
        }
        let page = self.clone();
        dialog.select_folder(Some(&parent), gtk::gio::Cancellable::NONE, move |result| {
            let Some(path) = result.ok().and_then(|file| file.path()) else {
                return;
            };
            page.set_project(Some(path.to_string_lossy().into_owned()));
        });
    }

    /// Run a write off the main thread, then redraw from the fresh scan it returns.
    fn mutate(
        &self,
        success: String,
        work: impl FnOnce(Option<String>) -> Result<Inventory, String> + Send + 'static,
    ) {
        self.mutate_reporting(move |project| work(project).map(|inv| (inv, success)));
    }

    /// `mutate` for a write whose message depends on what it did.
    fn mutate_reporting(
        &self,
        work: impl FnOnce(Option<String>) -> Result<(Inventory, String), String> + Send + 'static,
    ) {
        // The project now, not when the write gets its turn: a project picked meanwhile must
        // not receive a change clicked in the one before.
        let project = self.project.borrow().clone();
        let work: Write = Box::new(move || work(project));
        if self.busy.replace(true) {
            self.queued.borrow_mut().push_back(work);
            return;
        }
        self.run_write(work);
    }

    fn run_write(&self, work: Write) {
        // Supersede any scan in flight; its result would predate this write.
        let generation = self.generation.get().wrapping_add(1);
        self.generation.set(generation);
        let page = self.clone();
        gtk::glib::spawn_future_local(async move {
            let result = gtk::gio::spawn_blocking(work)
                .await
                .unwrap_or_else(|_| Err(crate::i18n::tr("Couldn't fix this")));
            if let Err(error) = &result {
                page.queued_error.borrow_mut().get_or_insert(error.clone());
            }
            let next = page.queued.borrow_mut().pop_front();
            if let Some(next) = next {
                // Redraw only after the last one, so the controls keep what was clicked.
                page.run_write(next);
                return;
            }
            page.busy.set(false);
            page.refresh_button.set_sensitive(true);
            let error = page.queued_error.borrow_mut().take();
            match (&result, &error) {
                (_, Some(error)) => page.show_feedback(error, true),
                (Ok((_, success)), None) => page.show_feedback(success, false),
                (Err(_), None) => {}
            }
            if page.refresh_pending.replace(false) {
                // A rescan or project change came in during the write: run it now, after the
                // write, and keep this write's message.
                page.rescan_keeping_feedback();
                return;
            }
            if generation != page.generation.get() {
                return;
            }
            match result {
                Ok((inv, _)) if error.is_none() => {
                    page.inventory.replace(Some(inv));
                    page.render();
                }
                // Part of a write may have landed; show what is on disk now.
                _ => page.rescan_keeping_feedback(),
            }
        });
    }

    fn render(&self) {
        let inventory = self.inventory.borrow();
        let Some(inv) = inventory.as_ref() else {
            return;
        };
        if inv.agents.is_empty() {
            self.screens.set_visible_child_name("empty");
            return;
        }
        self.screens.set_visible_child_name("content");
        let counts = [
            inv.skills
                .iter()
                .filter(|s| s.source != Source::Broken)
                .count(),
            inv.rules.len(),
            inv.issues.iter().filter(|i| !i.dismissed).count(),
        ];
        for ((tab, _, label), count) in self.tabs.iter().zip(counts) {
            label.set_label(&count.to_string());
            if *tab == Tab::Issues && count > 0 {
                label.add_css_class("warning");
            } else {
                label.remove_css_class("warning");
            }
        }
        let needle = self.search.text().trim().to_lowercase();
        match self.tab.get() {
            Tab::Skills => self.render_skills(inv, &needle),
            Tab::Rules => self.render_rules(inv, &needle),
            Tab::Issues => self.render_issues(inv),
        }
        if self.refocus.borrow_mut().take().is_some() {
            if let Some(widget) = self.refocus_widget.borrow_mut().take() {
                // After layout, so the new widget can take focus.
                gtk::glib::idle_add_local_once(move || {
                    widget.grab_focus();
                });
            }
        }
    }

    /// Redraw after a click on the control `key`, then give focus to its redrawn copy.
    fn render_keeping_focus(&self, key: String) {
        self.refocus.replace(Some(key));
        self.refocus_widget.replace(None);
        self.render();
    }

    /// Note `widget` as the redrawn copy of the control `key`, if focus should go back to it.
    fn offer_focus(&self, key: &str, widget: &impl IsA<gtk::Widget>) {
        if self.refocus.borrow().as_deref() == Some(key) {
            self.refocus_widget.replace(Some(widget.clone().upcast()));
        }
    }

    /// Redraw only the detail pane, keeping the list and its scroll position.
    fn render_details(&self) {
        let inventory = self.inventory.borrow();
        let Some(inv) = inventory.as_ref() else {
            return;
        };
        let needle = self.search.text().trim().to_lowercase();
        match self.tab.get() {
            Tab::Skills => {
                let visible = self.visible_skills(inv, &needle);
                self.fill_skill_detail(inv, self.current_skill(&visible));
            }
            Tab::Rules => {
                let visible = self.visible_rules(inv, &needle);
                self.fill_rule_detail(inv, self.current_rule(&visible));
            }
            Tab::Issues => {}
        }
    }

    fn visible_skills<'a>(&self, inv: &'a Inventory, needle: &str) -> Vec<&'a Skill> {
        let focus = self.focus.borrow();
        inv.skills
            .iter()
            .filter(|s| focus.as_ref().is_none_or(|id| s.states.contains_key(id)))
            .filter(|s| {
                needle.is_empty()
                    || s.name.to_lowercase().contains(needle)
                    || s.description.to_lowercase().contains(needle)
                    || s.path.to_lowercase().contains(needle)
            })
            .collect()
    }

    fn visible_rules<'a>(&self, inv: &'a Inventory, needle: &str) -> Vec<&'a RuleFile> {
        let focus = self.focus.borrow();
        inv.rules
            .iter()
            .filter(|r| focus.as_ref().is_none_or(|id| r.agents.contains_key(id)))
            .filter(|r| {
                needle.is_empty()
                    || r.name.to_lowercase().contains(needle)
                    || r.path.to_lowercase().contains(needle)
                    || r.preview.to_lowercase().contains(needle)
            })
            .collect()
    }

    fn current_skill<'a>(&self, visible: &[&'a Skill]) -> Option<&'a Skill> {
        let selected = self.selected_skill.borrow();
        selected
            .as_ref()
            .and_then(|key| visible.iter().find(|s| &s.key == key).copied())
            .or_else(|| visible.first().copied())
    }

    fn current_rule<'a>(&self, visible: &[&'a RuleFile]) -> Option<&'a RuleFile> {
        let selected = self.selected_rule.borrow();
        selected
            .as_ref()
            .and_then(|key| visible.iter().find(|r| &r.key == key).copied())
            .or_else(|| visible.first().copied())
    }

    fn render_banner(&self, pane: &Pane, inv: &Inventory) {
        clear_box(&pane.banner);
        let active: Vec<&Issue> = inv.issues.iter().filter(|i| !i.dismissed).collect();
        let Some(first) = active.first() else {
            return;
        };
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        card.add_css_class("toolport-skill-banner");
        card.append(&tile("dialog-warning-symbolic", "warning", 32));
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.set_hexpand(true);
        copy.set_valign(gtk::Align::Center);
        copy.append(&one_line(
            &if active.len() == 1 {
                crate::i18n::tr("1 thing needs a look")
            } else {
                crate::i18n::trf(
                    "{count} things need a look",
                    &[("count", &active.len().to_string())],
                )
            },
            &["heading"],
        ));
        copy.append(&one_line(&first.title, &["toolport-muted"]));
        card.append(&copy);
        let review = l10n::button("Review");
        review.set_valign(gtk::Align::Center);
        let page = self.clone();
        review.connect_clicked(move |_| page.select_tab(Tab::Issues));
        card.append(&review);
        pane.banner.append(&card);
    }

    /// The "SKILL … logos" row. Clicking a logo shows only what that agent loads.
    fn render_head(&self, pane: &Pane, agents: &[&Agent], label: &str, size_column: bool) {
        clear_box(&pane.head);
        let title = section(label);
        title.set_valign(gtk::Align::End);
        pane.head.append(&title);
        if size_column {
            let spacer = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            spacer.set_size_request(SIZE_COLUMN, 1);
            pane.head.append(&spacer);
        }
        let focus = self.focus.borrow().clone();
        for agent in agents {
            let overlay = gtk::Overlay::new();
            overlay.set_size_request(COLUMN, 22);
            let image = logo(&agent.id, 18);
            image.set_halign(gtk::Align::Center);
            image.set_valign(gtk::Align::Center);
            overlay.set_child(Some(&image));
            if !agent.verified {
                let pip = gtk::Box::new(gtk::Orientation::Horizontal, 0);
                pip.add_css_class("toolport-skill-pip");
                pip.set_halign(gtk::Align::End);
                pip.set_valign(gtk::Align::Start);
                overlay.add_overlay(&pip);
            }
            let button = gtk::Button::builder()
                .child(&overlay)
                .width_request(COLUMN)
                .css_classes(["flat", "toolport-skill-col"])
                .tooltip_text(if agent.verified {
                    agent.name.clone()
                } else {
                    crate::i18n::trf(
                        "{client}: folders from its docs, not checked on this machine",
                        &[("client", &agent.name)],
                    )
                })
                .build();
            // On Skills the filter keeps what the agent finds, switched off or not, so it can be
            // switched back on; on Rules it keeps what the agent reads.
            let values = [("client", agent.name.as_str())];
            button.update_property(&[gtk::accessible::Property::Label(&if size_column {
                crate::i18n::trf("Show only skills {client} finds", &values)
            } else {
                crate::i18n::trf("Show only what {client} loads", &values)
            })]);
            let pressed = focus.as_deref() == Some(agent.id.as_str());
            button.update_state(&[gtk::accessible::State::Pressed(tristate(pressed))]);
            match &focus {
                Some(id) if *id == agent.id => button.add_css_class("selected"),
                Some(_) => button.set_opacity(0.4),
                None => {}
            }
            let page = self.clone();
            let id = agent.id.clone();
            button.connect_clicked(move |_| {
                let next = if page.focus.borrow().as_deref() == Some(id.as_str()) {
                    None
                } else {
                    Some(id.clone())
                };
                page.focus.replace(next);
                page.render_keeping_focus(format!("column:{id}"));
            });
            self.offer_focus(&format!("column:{}", agent.id), &button);
            pane.head.append(&button);
        }
    }

    fn group_row(
        &self,
        key: String,
        icon: &str,
        label: &str,
        count: usize,
        note: &str,
    ) -> gtk::ListBoxRow {
        let open = !self.collapsed.borrow().contains(&key);
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        content.add_css_class("toolport-skill-group");
        let chevron = gtk::Image::from_icon_name(if open {
            "pan-down-symbolic"
        } else {
            "pan-end-symbolic"
        });
        chevron.add_css_class("toolport-muted");
        content.append(&chevron);
        let image = gtk::Image::from_icon_name(icon);
        image.add_css_class("toolport-muted");
        content.append(&image);
        let title = gtk::Label::new(Some(label));
        title.add_css_class("heading");
        content.append(&title);
        content.append(&{
            let pill = gtk::Label::new(Some(&count.to_string()));
            pill.add_css_class("toolport-skill-count");
            pill.add_css_class("caption");
            pill
        });
        if !note.is_empty() {
            content.append(&one_line(note, &["toolport-muted", "caption"]));
        }
        let row = gtk::ListBoxRow::builder()
            .child(&content)
            .selectable(false)
            .css_classes(["toolport-skill-group-row"])
            .build();
        row.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
        self.offer_focus(&format!("group:{key}"), &row);
        row
    }

    fn render_skills(&self, inv: &Inventory, needle: &str) {
        let pane = &self.skills;
        self.render_banner(pane, inv);
        let agents = skill_agents(inv);
        self.render_head(pane, &agents, &crate::i18n::tr("Skill"), true);
        let visible = self.visible_skills(inv, needle);
        // With nothing picked, the first row shown: groups in page order, collapsed ones skipped.
        let open = |source: Source| {
            !self.collapsed.borrow().contains(&source_key(source)) || !needle.is_empty()
        };
        let shown_first = SOURCE_ORDER
            .iter()
            .filter(|source| open(**source))
            .find_map(|source| visible.iter().find(|s| s.source == *source).copied());
        let current_skill = self
            .selected_skill
            .borrow()
            .as_ref()
            .and_then(|key| visible.iter().find(|s| &s.key == key).copied())
            .or(shown_first);
        let current = current_skill.map(|s| s.key.clone());

        let scroll = pane.scroll.vadjustment().value();
        self.rebuilding.set(true);
        clear_list(&pane.list);
        let mut rows = Vec::new();
        let mut selected_row = None;
        for source in SOURCE_ORDER {
            let items: Vec<&&Skill> = visible.iter().filter(|s| s.source == source).collect();
            if items.is_empty() {
                continue;
            }
            let key = source_key(source);
            // A search opens every group so no match hides in a collapsed one.
            let open = open(source);
            pane.list.append(&self.group_row(
                key.clone(),
                source_icon(source),
                &source_label(source),
                items.len(),
                "",
            ));
            rows.push(RowKind::Group(key));
            if !open {
                continue;
            }
            for skill in items {
                let row = self.skill_row(skill, &agents);
                if current.as_deref() == Some(skill.key.as_str()) {
                    selected_row = Some(row.clone());
                }
                pane.list.append(&row);
                rows.push(RowKind::Item(skill.key.clone()));
            }
        }
        if rows.is_empty() {
            let empty = muted(&if needle.is_empty() && self.focus.borrow().is_none() {
                crate::i18n::tr("No skills found in any agent's folders.")
            } else {
                crate::i18n::tr("No skills match.")
            });
            empty.set_halign(gtk::Align::Center);
            empty.set_margin_top(32);
            pane.list.append(
                &gtk::ListBoxRow::builder()
                    .child(&empty)
                    .selectable(false)
                    .activatable(false)
                    .build(),
            );
            rows.push(RowKind::Group(String::new()));
        }
        pane.rows.replace(rows);
        pane.list.select_row(selected_row.as_ref());
        self.rebuilding.set(false);
        restore_scroll(&pane.scroll, scroll);
        self.fill_skill_detail(inv, current_skill);
    }

    fn skill_row(&self, skill: &Skill, agents: &[&Agent]) -> gtk::ListBoxRow {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.add_css_class("toolport-skill-row");
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.set_hexpand(true);
        let name = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        let title = one_line(&skill.name, &["toolport-skill-name"]);
        title.set_hexpand(false);
        name.append(&title);
        if skill.description_chars > crate::skills::DESCRIPTION_CAP {
            let warn = gtk::Image::from_icon_name("dialog-warning-symbolic");
            warn.add_css_class("approval-warning");
            warn.set_tooltip_text(Some(&crate::i18n::tr(
                "Description is cut off in Claude Code",
            )));
            name.append(&warn);
        }
        copy.append(&name);
        copy.append(&one_line(
            if skill.source == Source::Broken {
                &skill.path
            } else {
                &skill.description
            },
            &["toolport-muted", "caption"],
        ));
        content.append(&copy);
        let size = gtk::Label::builder()
            .label(if skill.source == Source::Broken {
                String::new()
            } else {
                fmt_bytes(skill.size_bytes)
            })
            .width_request(SIZE_COLUMN)
            .xalign(1.0)
            .margin_end(6)
            .css_classes(["toolport-muted", "caption", "numeric"])
            .build();
        content.append(&size);
        let dots = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let focus = self.focus.borrow();
        for agent in agents {
            let state = skill.states.get(&agent.id).map(|s| s.state);
            let dot = state_dot(
                state,
                &crate::i18n::trf(
                    "{client}: {state}",
                    &[("client", &agent.name), ("state", &state_label(state))],
                ),
            );
            if focus.as_ref().is_some_and(|id| *id != agent.id) {
                dot.set_opacity(0.35);
            }
            dots.append(&dot);
        }
        content.append(&dots);
        gtk::ListBoxRow::builder().child(&content).build()
    }

    /// `target` is the absolute key the backend allows; `path` is what the user sees and copies.
    /// Open uses the app picked from its menu, else the system default.
    fn file_actions(
        &self,
        target: &str,
        path: &str,
        open_label: &'static str,
        can_open: bool,
    ) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 8);
        let apps = gtk::Box::new(gtk::Orientation::Vertical, 2);
        apps.set_margin_top(4);
        apps.set_margin_bottom(4);
        let popover = gtk::Popover::builder().child(&apps).build();
        let open = adw::SplitButton::builder()
            .child(&l10n::label(open_label))
            .popover(&popover)
            .build();
        open.set_dropdown_tooltip(&crate::i18n::tr("Open with"));
        {
            let page = self.clone();
            let target = target.to_string();
            let shown = path.to_string();
            open.connect_clicked(move |_| page.open_file(&target, &shown, None, false));
        }
        {
            // Listed when the menu opens, so a newly installed editor shows up.
            let page = self.clone();
            let target = target.to_string();
            let shown = path.to_string();
            // The handler gets the popover as its argument: holding a strong copy here would keep
            // every redrawn detail pane's menu alive.
            popover.connect_show(move |popover| {
                clear_box(&apps);
                let title = section(&crate::i18n::tr("Open with"));
                title.set_margin_start(10);
                title.set_margin_bottom(4);
                apps.append(&title);
                let found = crate::skills::open_apps(&target);
                if found.is_empty() {
                    let none = muted(&crate::i18n::tr("No other apps found"));
                    none.set_margin_start(10);
                    none.set_margin_end(10);
                    apps.append(&none);
                }
                let current = found
                    .iter()
                    .find(|app| app.chosen)
                    .or_else(|| found.iter().find(|app| app.default))
                    .map(|app| app.id.clone());
                for app in found {
                    let line = gtk::Box::new(gtk::Orientation::Horizontal, 8);
                    let check = gtk::Image::from_icon_name("object-select-symbolic");
                    check.set_opacity(if current.as_deref() == Some(app.id.as_str()) {
                        1.0
                    } else {
                        0.0
                    });
                    line.append(&check);
                    line.append(&gtk::Label::new(Some(&if app.default {
                        crate::i18n::trf("{app} (default)", &[("app", &app.name)])
                    } else {
                        app.name.clone()
                    })));
                    let button = gtk::Button::builder()
                        .child(&line)
                        .css_classes(["flat"])
                        .accessible_role(gtk::AccessibleRole::MenuItemRadio)
                        .build();
                    button.update_state(&[gtk::accessible::State::Checked(tristate(
                        current.as_deref() == Some(app.id.as_str()),
                    ))]);
                    let page = page.clone();
                    let target = target.clone();
                    let shown = shown.clone();
                    let popover = popover.downgrade();
                    button.connect_clicked(move |_| {
                        if let Some(popover) = popover.upgrade() {
                            popover.popdown();
                        }
                        page.open_file(&target, &shown, Some(app.id.clone()), false);
                    });
                    apps.append(&button);
                }
            });
        }
        let reveal = l10n::button("Reveal");
        {
            let page = self.clone();
            let target = target.to_string();
            let shown = path.to_string();
            reveal.connect_clicked(move |_| page.open_file(&target, &shown, None, true));
        }
        let copy = gtk::Button::from_icon_name("edit-copy-symbolic");
        copy.set_tooltip_text(Some(&crate::i18n::tr("Copy path")));
        l10n::bind_tooltip(&copy, "Copy path");
        {
            let path = path.to_string();
            let page = self.clone();
            copy.connect_clicked(move |button| {
                button.clipboard().set_text(&path);
                page.show_feedback(&crate::i18n::tr("Path copied"), false);
            });
        }
        if can_open {
            row.append(&open);
        }
        row.append(&reveal);
        row.append(&copy);
        row
    }

    /// Open a file (in `app` when given, which also becomes the app Open uses) or its folder.
    fn open_file(&self, target: &str, shown: &str, app: Option<String>, reveal: bool) {
        let project = self.project.borrow().clone();
        let target = target.to_string();
        let shown = shown.to_string();
        let page = self.clone();
        gtk::glib::spawn_future_local(async move {
            let result = gtk::gio::spawn_blocking(move || match app {
                Some(app) => crate::skills::open_path_with(&target, &app, project.as_deref()),
                None => crate::skills::open_path(&target, reveal, project.as_deref()),
            })
            .await;
            if let Ok(Err(error)) = result {
                page.show_feedback(
                    &crate::i18n::trf(
                        "Couldn't open {path}: {error}",
                        &[("path", &shown), ("error", &error)],
                    ),
                    true,
                );
            }
        });
    }

    /// Ask, then move a skill to the trash and remove the links that reach it.
    fn confirm_remove(&self, skill: &Skill) {
        let Some(parent) = self.app.active_window() else {
            return;
        };
        let heading = crate::i18n::trf("Remove {skill}?", &[("skill", &skill.name)]);
        // Say what will happen: a folder in a skills folder goes to the trash; one that lives
        // elsewhere (a repo) only loses the links to it.
        let folder = own_folder(skill);
        let body = if skill.trash_on_remove {
            crate::i18n::tr(
                "Its folder goes to the trash and links to it are removed, so no client loads it. You can restore the folder from the trash.",
            )
        } else {
            crate::i18n::trf(
                "Only the links to it are removed, so no client loads it. Its folder at {path} stays where it is.",
                &[("path", folder)],
            )
        };
        #[allow(deprecated)]
        let dialog = adw::MessageDialog::new(Some(&parent), Some(&heading), Some(&body));
        dialog.add_response("cancel", &crate::i18n::tr("Cancel"));
        l10n::bind_dialog_response(&dialog, "cancel", "Cancel");
        dialog.add_response("remove", &crate::i18n::tr("Remove"));
        l10n::bind_dialog_response(&dialog, "remove", "Remove");
        dialog.set_close_response("cancel");
        dialog.set_default_response(Some("cancel"));
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        let page = self.clone();
        let key = skill.key.clone();
        let name = skill.name.clone();
        dialog.connect_response(None, move |dialog, response| {
            if response == "remove" {
                let key = key.clone();
                let name = name.clone();
                page.mutate_reporting(move |project| {
                    let removed =
                        crate::skills::remove_skill(&key, project.as_deref()).map_err(titled(
                            crate::i18n::trf("Couldn't remove {skill}", &[("skill", &name)]),
                        ))?;
                    let message = if removed.trashed.is_some() {
                        crate::i18n::trf("Moved {skill} to the trash", &[("skill", &name)])
                    } else {
                        crate::i18n::trf(
                            "Removed the links to {skill}. Its folder stays where it is.",
                            &[("skill", &name)],
                        )
                    };
                    Ok((removed.inventory, message))
                });
            }
            dialog.close();
        });
        dialog.present();
    }

    fn detail_header(&self, icon: &str, name: &str, badges: &[gtk::Widget]) -> gtk::Box {
        let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        header.append(&tile(icon, "accent", 44));
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 6);
        copy.set_hexpand(true);
        let title = heading(name);
        title.add_css_class("title-3");
        copy.append(&title);
        let chips = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for badge in badges {
            chips.append(badge);
        }
        copy.append(&chips);
        header.append(&copy);
        header
    }

    fn fill_skill_detail(&self, inv: &Inventory, skill: Option<&Skill>) {
        let pane = &self.skills;
        let scroll = pane.detail_scroll.vadjustment().value();
        clear_box(&pane.detail);
        self.controls.borrow_mut().clear();
        let Some(skill) = skill else {
            let empty = muted(&crate::i18n::tr("Pick a skill to see where it loads."));
            empty.set_halign(gtk::Align::Center);
            empty.set_margin_top(32);
            pane.detail.append(&empty);
            return;
        };
        let broken = skill.source == Source::Broken;
        let mut badges: Vec<gtk::Widget> = vec![icon_badge(
            source_icon(skill.source),
            &source_label(skill.source),
            if broken { "error" } else { "review" },
        )
        .upcast()];
        if let Some(repo) = &skill.git_repo {
            badges.push(
                icon_badge(
                    "media-playlist-shuffle-symbolic",
                    &crate::i18n::trf("git repo: {name}", &[("name", repo)]),
                    "",
                )
                .upcast(),
            );
        }
        if !skill.editable && !broken {
            badges.push(badge(&crate::i18n::tr("Read only"), "").upcast());
        }
        pane.detail
            .append(&self.detail_header("starred-symbolic", &skill.name, &badges));
        // A link to a missing folder has nothing to open, only a place to reveal.
        let actions = self.file_actions(&skill.key, &skill.path, "Open SKILL.md", !broken);
        if skill.removable {
            let remove = gtk::Button::from_icon_name("user-trash-symbolic");
            let label = crate::i18n::trf("Remove {skill}", &[("skill", &skill.name)]);
            remove.set_tooltip_text(Some(&label));
            remove.update_property(&[gtk::accessible::Property::Label(&label)]);
            remove.set_sensitive(!self.busy.get());
            let page = self.clone();
            let skill = skill.clone();
            remove.connect_clicked(move |_| page.confirm_remove(&skill));
            actions.append(&remove);
        }
        pane.detail.append(&actions);

        if skill.source != Source::Broken {
            pane.detail
                .append(&section(&crate::i18n::tr("Description")));
            let description = if skill.description.is_empty() {
                muted(&crate::i18n::tr("No description"))
            } else {
                let label = muted(&skill.description);
                label.remove_css_class("toolport-muted");
                // Long descriptions stop after a few lines, like the mockup; Open shows the rest.
                label.set_lines(6);
                label.set_ellipsize(gtk::pango::EllipsizeMode::End);
                label.set_tooltip_text(Some(&skill.description));
                label
            };
            description.set_selectable(true);
            pane.detail.append(&description);
            let cap = crate::skills::DESCRIPTION_CAP;
            let bar = gtk::LevelBar::builder()
                .min_value(0.0)
                .max_value(cap as f64)
                .value(skill.description_chars.min(cap) as f64)
                .css_classes(["toolport-skill-meter"])
                .build();
            bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_LOW));
            bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_HIGH));
            bar.remove_offset_value(Some(gtk::LEVEL_BAR_OFFSET_FULL));
            if skill.description_chars > cap {
                bar.add_css_class("over");
            }
            bar.update_property(&[gtk::accessible::Property::Label(&crate::i18n::tr(
                "Description length",
            ))]);
            pane.detail.append(&bar);
            let counts = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            let used = muted(&crate::i18n::trf(
                "{count} of {cap} characters",
                &[
                    ("count", &crate::i18n::group_digits(skill.description_chars)),
                    ("cap", &crate::i18n::group_digits(cap)),
                ],
            ));
            used.add_css_class("caption");
            used.set_hexpand(true);
            counts.append(&used);
            let limit = muted(&crate::i18n::tr("Claude Code's limit"));
            limit.add_css_class("caption");
            limit.set_wrap(false);
            limit.set_hexpand(false);
            counts.append(&limit);
            pane.detail.append(&counts);
        }

        pane.detail.append(&section(&crate::i18n::tr("File")));
        let fields = gtk::Grid::builder()
            .column_spacing(16)
            .row_spacing(8)
            .build();
        let mut row = 0;
        let mut field = |label: &str, value: gtk::Widget| {
            let name = muted(label);
            name.set_wrap(false);
            name.set_hexpand(false);
            name.set_valign(gtk::Align::Start);
            fields.attach(&name, 0, row, 1, 1);
            value.set_hexpand(true);
            fields.attach(&value, 1, row, 1, 1);
            row += 1;
        };
        field(&crate::i18n::tr("Path"), path_value(&skill.path).upcast());
        // `links` holds folders, the skill's own among them; only the other ways in are news.
        let own_folder = own_folder(skill);
        let others: Vec<&str> = skill
            .links
            .iter()
            .map(String::as_str)
            .filter(|l| *l != skill.path && *l != own_folder)
            .collect();
        if !others.is_empty() {
            field(
                &crate::i18n::tr("Reached by"),
                path_value(&others.join("\n")).upcast(),
            );
        }
        field(
            &crate::i18n::tr("Source"),
            value_label(&skill.source_note).upcast(),
        );
        if skill.source != Source::Broken {
            field(
                &crate::i18n::tr("Size"),
                value_label(&if skill.lines == 1 {
                    crate::i18n::trf("{size} · 1 line", &[("size", &fmt_bytes(skill.size_bytes))])
                } else {
                    crate::i18n::trf(
                        "{size} · {lines} lines",
                        &[
                            ("size", &fmt_bytes(skill.size_bytes)),
                            ("lines", &crate::i18n::group_digits(skill.lines)),
                        ],
                    )
                })
                .upcast(),
            );
        }
        if let Some(changed) = skill.modified_secs.and_then(fmt_date) {
            field(&crate::i18n::tr("Changed"), value_label(&changed).upcast());
        }
        pane.detail.append(&fields);

        let agents = skill_agents(inv);
        let loaded = agents
            .iter()
            .filter(|a| {
                skill
                    .states
                    .get(&a.id)
                    .is_some_and(|s| matches!(s.state, State::On | State::NameOnly))
            })
            .count();
        pane.detail.append(&section_with(
            &crate::i18n::tr("Agents"),
            &crate::i18n::trf(
                "{count} of {total} load it",
                &[
                    ("count", &loaded.to_string()),
                    ("total", &agents.len().to_string()),
                ],
            ),
        ));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("toolport-skill-agents");
        for agent in agents {
            list.append(&self.agent_row(agent, skill));
        }
        pane.detail.append(&list);
        if !skill.preview.is_empty() {
            pane.detail.append(&section(&crate::i18n::tr("Preview")));
            pane.detail.append(&preview_label(&skill.preview));
        }
        restore_scroll(&pane.detail_scroll, scroll);
        // A write redraws the pane; put focus back on the control the user just used.
        if let Some(agent) = self.last_control.borrow_mut().take() {
            let choice = self
                .focused_choice
                .borrow()
                .clone()
                .filter(|key| key.starts_with(&format!("{agent}#")));
            let controls = self.controls.borrow();
            if let Some(control) = choice
                .and_then(|key| controls.get(&key))
                .or_else(|| controls.get(&agent))
            {
                control.grab_focus();
            }
        }
    }

    fn agent_row(&self, agent: &Agent, skill: &Skill) -> gtk::Box {
        let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        row.add_css_class("toolport-skill-agent");
        row.append(&logo_tile(&agent.id));
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.set_hexpand(true);
        let name = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        name.append(&heading(&agent.name));
        if !agent.verified {
            let docs = badge(&crate::i18n::tr("from docs"), "disabled");
            docs.set_tooltip_text(Some(&crate::i18n::tr(
                "Folders come from this client's docs, not a check on this machine",
            )));
            name.append(&docs);
        }
        copy.append(&name);
        let state = skill.states.get(&agent.id);
        let via = muted(&state.map_or_else(
            || crate::i18n::tr("Not in any folder it reads"),
            |s| s.via.clone(),
        ));
        via.add_css_class("caption");
        copy.append(&via);
        row.append(&copy);
        let control = self.state_control(agent, skill);
        control.set_valign(gtk::Align::Center);
        row.append(&control);
        if state.is_none() {
            row.add_css_class("dim");
        }
        row
    }

    fn state_control(&self, agent: &Agent, skill: &Skill) -> gtk::Widget {
        let current = skill.states.get(&agent.id).map(|s| s.state);
        let Some(current) = current else {
            let label = muted(&state_label(None));
            label.set_wrap(false);
            label.set_halign(gtk::Align::End);
            label.add_css_class("caption");
            return label.upcast();
        };
        match current {
            State::Broken => return badge(&state_label(Some(current)), "error").upcast(),
            State::Shadowed => {
                return badge(&state_label(Some(current)), "approval-warning").upcast()
            }
            _ => {}
        }
        if !agent.can_toggle {
            let pill = icon_badge(
                if current == State::Off {
                    "action-unavailable-symbolic"
                } else {
                    "media-record-symbolic"
                },
                &state_label(Some(current)),
                if current == State::Off {
                    "disabled"
                } else {
                    "success"
                },
            );
            pill.set_tooltip_text(Some(&agent.switch_note));
            return pill.upcast();
        }
        let set = {
            let page = self.clone();
            let agent_id = agent.id.clone();
            let agent_name = agent.name.clone();
            let key = skill.key.clone();
            let skill_name = skill.name.clone();
            move |wanted: Wanted| {
                let success = match wanted {
                    Wanted::Off => crate::i18n::trf(
                        "Turned off {skill} in {client}",
                        &[("skill", &skill_name), ("client", &agent_name)],
                    ),
                    Wanted::NameOnly => crate::i18n::trf(
                        "{client} now lists {skill} by name only",
                        &[("skill", &skill_name), ("client", &agent_name)],
                    ),
                    Wanted::On => crate::i18n::trf(
                        "Turned on {skill} in {client}",
                        &[("skill", &skill_name), ("client", &agent_name)],
                    ),
                };
                page.last_control.replace(Some(agent_id.clone()));
                let agent_id = agent_id.clone();
                let key = key.clone();
                let failed = titled(crate::i18n::trf(
                    "Couldn't change {skill} in {client}",
                    &[("skill", &skill_name), ("client", &agent_name)],
                ));
                page.mutate(success, move |project| {
                    crate::skills::set_skill_state(&agent_id, &key, wanted, project.as_deref())
                        .map_err(failed)
                });
            }
        };
        if agent.name_only {
            // On / Name only / Off as one segmented control, like the React radio group.
            let group = gtk::Box::new(gtk::Orientation::Horizontal, 0);
            group.add_css_class("linked");
            group.add_css_class("toolport-skill-segments");
            group.update_property(&[gtk::accessible::Property::Label(&crate::i18n::trf(
                "{skill} in {client}",
                &[("skill", &skill.name), ("client", &agent.name)],
            ))]);
            let set = Rc::new(set);
            let mut first: Option<gtk::ToggleButton> = None;
            for (wanted, state, title) in [
                (Wanted::On, State::On, crate::i18n::tr("On")),
                (
                    Wanted::NameOnly,
                    State::NameOnly,
                    crate::i18n::tr("Name only"),
                ),
                (Wanted::Off, State::Off, crate::i18n::tr("Off")),
            ] {
                let button = gtk::ToggleButton::builder()
                    .label(title)
                    .active(current == state)
                    .css_classes(["caption"])
                    .build();
                if let Some(first) = &first {
                    button.set_group(Some(first));
                } else {
                    first = Some(button.clone());
                    self.controls
                        .borrow_mut()
                        .insert(agent.id.clone(), button.clone().upcast());
                }
                if current == state {
                    self.controls
                        .borrow_mut()
                        .insert(agent.id.clone(), button.clone().upcast());
                }
                let slot = format!("{}#{}", agent.id, state_label(Some(state)));
                self.controls
                    .borrow_mut()
                    .insert(slot.clone(), button.clone().upcast());
                let watch = gtk::EventControllerFocus::new();
                let focused = self.focused_choice.clone();
                watch.connect_enter(move |_| {
                    focused.replace(Some(slot.clone()));
                });
                button.add_controller(watch);
                let set = set.clone();
                // Clicking back to the saved state still writes: an earlier click may be queued.
                button.connect_toggled(move |button| {
                    if button.is_active() {
                        set(wanted);
                    }
                });
                group.append(&button);
            }
            return group.upcast();
        }
        let name = crate::i18n::trf(
            "Load {skill} in {client}",
            &[("skill", &skill.name), ("client", &agent.name)],
        );
        let switch = gtk::Switch::builder()
            .active(current != State::Off)
            .valign(gtk::Align::Center)
            .tooltip_text(&name)
            .build();
        switch.update_property(&[gtk::accessible::Property::Label(&name)]);
        self.controls
            .borrow_mut()
            .insert(agent.id.clone(), switch.clone().upcast());
        switch.connect_state_set(move |_, active| {
            set(if active { Wanted::On } else { Wanted::Off });
            gtk::glib::Propagation::Proceed
        });
        switch.upcast()
    }

    fn render_rules(&self, inv: &Inventory, needle: &str) {
        let pane = &self.rules;
        self.render_banner(pane, inv);
        let agents: Vec<&Agent> = inv.agents.iter().collect();
        self.render_head(pane, &agents, &crate::i18n::tr("Rules file"), false);
        let visible = self.visible_rules(inv, needle);
        let current = self.current_rule(&visible).map(|r| r.key.clone());

        let scroll = pane.scroll.vadjustment().value();
        self.rebuilding.set(true);
        clear_list(&pane.list);
        let mut rows = Vec::new();
        let mut selected_row = None;
        let project = self.project.borrow().clone();
        for (scope, label, icon) in [
            (
                "global",
                crate::i18n::tr("Global"),
                "avatar-default-symbolic",
            ),
            (
                "project",
                crate::i18n::tr("Project"),
                "folder-open-symbolic",
            ),
        ] {
            let items: Vec<&&RuleFile> = visible.iter().filter(|r| r.scope == scope).collect();
            if items.is_empty() {
                continue;
            }
            let key = format!("rules:{scope}");
            let open = !self.collapsed.borrow().contains(&key) || !needle.is_empty();
            let note = if scope == "global" {
                crate::i18n::tr("every session")
            } else {
                project.as_deref().map(folder_name).unwrap_or_default()
            };
            pane.list
                .append(&self.group_row(key.clone(), icon, &label, items.len(), &note));
            rows.push(RowKind::Group(key));
            if !open {
                continue;
            }
            for rule in items {
                let row = self.rule_row(rule, &agents);
                if current.as_deref() == Some(rule.key.as_str()) {
                    selected_row = Some(row.clone());
                }
                pane.list.append(&row);
                rows.push(RowKind::Item(rule.key.clone()));
            }
        }
        if rows.is_empty() {
            let empty = muted(&if needle.is_empty() && self.focus.borrow().is_none() {
                crate::i18n::tr("No rules files found.")
            } else {
                crate::i18n::tr("No rules files match.")
            });
            empty.set_halign(gtk::Align::Center);
            empty.set_margin_top(32);
            pane.list.append(
                &gtk::ListBoxRow::builder()
                    .child(&empty)
                    .selectable(false)
                    .activatable(false)
                    .build(),
            );
            rows.push(RowKind::Group(String::new()));
        }
        pane.rows.replace(rows);
        pane.list.select_row(selected_row.as_ref());
        self.rebuilding.set(false);
        restore_scroll(&pane.scroll, scroll);
        self.fill_rule_detail(inv, self.current_rule(&visible));
    }

    fn rule_row(&self, rule: &RuleFile, agents: &[&Agent]) -> gtk::ListBoxRow {
        let content = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        content.add_css_class("toolport-skill-row");
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
        copy.set_hexpand(true);
        copy.append(&one_line(&rule.name, &["toolport-skill-name"]));
        copy.append(&one_line(
            &rule.path,
            &["toolport-muted", "caption", "monospace"],
        ));
        content.append(&copy);
        let dots = gtk::Box::new(gtk::Orientation::Horizontal, 0);
        let focus = self.focus.borrow();
        for agent in agents {
            let loaded = rule.agents.contains_key(&agent.id);
            let dot = state_dot(
                loaded.then_some(State::On),
                &crate::i18n::trf(
                    "{client}: {state}",
                    &[
                        ("client", &agent.name),
                        (
                            "state",
                            &if loaded {
                                crate::i18n::tr("Loaded")
                            } else {
                                crate::i18n::tr("Not loaded")
                            },
                        ),
                    ],
                ),
            );
            if focus.as_ref().is_some_and(|id| *id != agent.id) {
                dot.set_opacity(0.35);
            }
            dots.append(&dot);
        }
        content.append(&dots);
        gtk::ListBoxRow::builder().child(&content).build()
    }

    fn fill_rule_detail(&self, inv: &Inventory, rule: Option<&RuleFile>) {
        let pane = &self.rules;
        let scroll = pane.detail_scroll.vadjustment().value();
        clear_box(&pane.detail);
        let Some(rule) = rule else {
            let empty = muted(&crate::i18n::tr(
                "Pick a rules file to see which agents read it.",
            ));
            empty.set_halign(gtk::Align::Center);
            empty.set_margin_top(32);
            pane.detail.append(&empty);
            return;
        };
        let scope = badge(
            &if rule.scope == "project" {
                crate::i18n::tr("Project")
            } else {
                crate::i18n::tr("Global")
            },
            "",
        );
        pane.detail.append(&self.detail_header(
            "text-x-generic-symbolic",
            &rule.name,
            &[scope.upcast()],
        ));
        pane.detail
            .append(&self.file_actions(&rule.key, &rule.path, "Open file", true));
        pane.detail.append(&section(&crate::i18n::tr("File")));
        let fields = gtk::Grid::builder()
            .column_spacing(16)
            .row_spacing(8)
            .build();
        for (row, (label, value)) in [
            (
                crate::i18n::tr("Path"),
                path_value(&rule.path).upcast::<gtk::Widget>(),
            ),
            (
                crate::i18n::tr("Size"),
                value_label(&fmt_bytes(rule.size_bytes)).upcast(),
            ),
        ]
        .into_iter()
        .enumerate()
        {
            let name = muted(&label);
            name.set_wrap(false);
            name.set_hexpand(false);
            name.set_valign(gtk::Align::Start);
            fields.attach(&name, 0, row as i32, 1, 1);
            value.set_hexpand(true);
            fields.attach(&value, 1, row as i32, 1, 1);
        }
        pane.detail.append(&fields);

        let loaded = inv
            .agents
            .iter()
            .filter(|a| rule.agents.contains_key(&a.id))
            .count();
        pane.detail.append(&section_with(
            &crate::i18n::tr("Agents"),
            &crate::i18n::trf(
                "{count} of {total} load it",
                &[
                    ("count", &loaded.to_string()),
                    ("total", &inv.agents.len().to_string()),
                ],
            ),
        ));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("toolport-skill-agents");
        for agent in &inv.agents {
            let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
            row.add_css_class("toolport-skill-agent");
            row.append(&logo_tile(&agent.id));
            let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
            copy.set_hexpand(true);
            copy.append(&heading(&agent.name));
            match rule.agents.get(&agent.id) {
                Some(chain) => {
                    for (index, step) in chain.iter().enumerate() {
                        let text = match step.kind.as_str() {
                            "file" => step.path.clone(),
                            "link" => crate::i18n::trf("links to {path}", &[("path", &step.path)]),
                            "import" => {
                                crate::i18n::trf("@imports {path}", &[("path", &step.path)])
                            }
                            _ => crate::i18n::trf("skips @import {path}", &[("path", &step.path)]),
                        };
                        let line = gtk::Box::new(gtk::Orientation::Horizontal, 6);
                        if index > 0 {
                            let arrow = gtk::Label::new(Some("↳"));
                            arrow.add_css_class("toolport-muted");
                            arrow.set_valign(gtk::Align::Start);
                            line.append(&arrow);
                        }
                        let label = muted(&text);
                        label.add_css_class("caption");
                        if step.kind == "file" {
                            label.add_css_class("monospace");
                        }
                        if step.kind == "skipped-import" {
                            label.add_css_class("warning");
                        }
                        line.append(&label);
                        copy.append(&line);
                    }
                    row.append(&copy);
                    let loaded = icon_badge(
                        "media-record-symbolic",
                        &crate::i18n::tr("Loaded"),
                        "success",
                    );
                    loaded.set_valign(gtk::Align::Start);
                    row.append(&loaded);
                }
                None => {
                    let note = muted(&agent.rules_note.clone().unwrap_or_else(|| {
                        let others: Vec<&str> = agent
                            .rule_locations
                            .iter()
                            .map(String::as_str)
                            .filter(|path| *path != rule.path)
                            .collect();
                        if others.is_empty() {
                            crate::i18n::tr("No global rules file")
                        } else {
                            crate::i18n::trf(
                                "Reads {paths}, which doesn't lead here",
                                &[("paths", &others.join(", "))],
                            )
                        }
                    }));
                    note.add_css_class("caption");
                    copy.append(&note);
                    row.append(&copy);
                    let state = muted(&crate::i18n::tr("Not loaded"));
                    state.set_wrap(false);
                    state.set_hexpand(false);
                    state.set_valign(gtk::Align::Start);
                    state.add_css_class("caption");
                    row.append(&state);
                    row.add_css_class("dim");
                }
            }
            list.append(&row);
        }
        pane.detail.append(&list);
        if !rule.preview.is_empty() {
            pane.detail.append(&section(&crate::i18n::tr("Preview")));
            pane.detail.append(&preview_label(&rule.preview));
        }
        restore_scroll(&pane.detail_scroll, scroll);
    }

    fn render_issues(&self, inv: &Inventory) {
        clear_box(&self.issues_list);
        let (dismissed, active): (Vec<&Issue>, Vec<&Issue>) =
            inv.issues.iter().partition(|i| i.dismissed);
        if active.is_empty() {
            let empty = gtk::Box::new(gtk::Orientation::Vertical, 6);
            empty.set_margin_top(40);
            empty.set_halign(gtk::Align::Center);
            let title = heading(&crate::i18n::tr("Nothing needs a look"));
            title.set_halign(gtk::Align::Center);
            empty.append(&title);
            empty.append(&muted(&crate::i18n::tr(
                "Every skill and rules file loads the way it's set.",
            )));
            self.issues_list.append(&empty);
        }
        for issue in active {
            self.issues_list.append(&self.issue_card(inv, issue));
        }
        if !dismissed.is_empty() {
            let open = !self.collapsed.borrow().contains(DISMISSED_KEY);
            let content = gtk::Box::new(gtk::Orientation::Horizontal, 8);
            content.add_css_class("toolport-skill-group");
            let chevron = gtk::Image::from_icon_name(if open {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            });
            chevron.add_css_class("toolport-muted");
            content.append(&chevron);
            let title = gtk::Label::new(Some(&crate::i18n::tr("Dismissed")));
            title.add_css_class("heading");
            content.append(&title);
            let pill = gtk::Label::new(Some(&dismissed.len().to_string()));
            pill.add_css_class("toolport-skill-count");
            pill.add_css_class("caption");
            content.append(&pill);
            content.append(&one_line(
                &crate::i18n::tr("Back if anything about them changes"),
                &["toolport-muted", "caption"],
            ));
            let toggle = gtk::Button::builder()
                .child(&content)
                .css_classes(["flat"])
                .build();
            toggle.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
            let page = self.clone();
            toggle.connect_clicked(move |_| {
                {
                    let mut collapsed = page.collapsed.borrow_mut();
                    if !collapsed.remove(DISMISSED_KEY) {
                        collapsed.insert(DISMISSED_KEY.to_string());
                    }
                }
                page.render_keeping_focus(DISMISSED_KEY.to_string());
            });
            self.offer_focus(DISMISSED_KEY, &toggle);
            self.issues_list.append(&toggle);
            if open {
                for issue in dismissed {
                    let card = self.issue_card(inv, issue);
                    card.set_opacity(0.7);
                    self.issues_list.append(&card);
                }
            }
        }
        clear_box(&self.issues_side);
        self.issues_side
            .append(&section(&crate::i18n::tr("By agent")));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        list.add_css_class("toolport-skill-agents");
        for agent in &inv.agents {
            list.append(&agent_summary(inv, agent));
        }
        self.issues_side.append(&list);
        let note = muted(&crate::i18n::tr(
            "\"Own switch\" means the agent has its own per-skill setting that Toolport can write. Toolport never moves or deletes your skill folders to turn a skill off.",
        ));
        note.add_css_class("caption");
        self.issues_side.append(&note);
    }

    fn issue_card(&self, inv: &Inventory, issue: &Issue) -> gtk::Box {
        let card = gtk::Box::new(gtk::Orientation::Horizontal, 12);
        card.add_css_class("toolport-skill-issue");
        let (icon, class) = match issue.level.as_str() {
            "error" => ("dialog-error-symbolic", "error"),
            "warn" => ("dialog-warning-symbolic", "warning"),
            _ => ("dialog-information-symbolic", "info"),
        };
        card.append(&tile(icon, class, 32));
        let copy = gtk::Box::new(gtk::Orientation::Vertical, 6);
        copy.set_hexpand(true);
        let top = gtk::Box::new(gtk::Orientation::Horizontal, 10);
        let title = heading(&issue.title);
        title.set_hexpand(true);
        // A wrapping label asks for a short line by default; ask for one line and wrap only
        // when the row is too narrow.
        title.set_natural_wrap_mode(gtk::NaturalWrapMode::None);
        top.append(&title);
        let dismiss = if issue.dismissed {
            l10n::button("Show again")
        } else {
            l10n::button("Dismiss")
        };
        dismiss.add_css_class("flat");
        dismiss.set_valign(gtk::Align::Start);
        dismiss.set_sensitive(!self.busy.get());
        {
            let page = self.clone();
            let fingerprint = issue.fingerprint.clone();
            let hide = !issue.dismissed;
            dismiss.connect_clicked(move |_| {
                let fingerprint = fingerprint.clone();
                // A dismissed card folds into the Dismissed section; one brought back returns to
                // the list with its Dismiss button focused.
                page.refocus.replace(Some(if hide {
                    DISMISSED_KEY.to_string()
                } else {
                    format!("issue:{fingerprint}")
                }));
                let failed = titled(crate::i18n::tr("Couldn't change this"));
                page.mutate(String::new(), move |project| {
                    crate::skills::dismiss_issue(&fingerprint, hide, project.as_deref())
                        .map_err(failed)
                });
            });
        }
        self.offer_focus(&format!("issue:{}", issue.fingerprint), &dismiss);
        top.append(&dismiss);
        if let Some(label) = issue.fix_label.as_ref().filter(|_| !issue.dismissed) {
            let button = gtk::Button::with_label(label);
            button.add_css_class("suggested-action");
            button.set_valign(gtk::Align::Start);
            button.set_sensitive(!self.busy.get());
            let page = self.clone();
            let id = issue.id.clone();
            let fingerprint = issue.fingerprint.clone();
            let title = issue.title.clone();
            let label = label.clone();
            button.connect_clicked(move |_| page.confirm_fix(&id, &fingerprint, &title, &label));
            top.append(&button);
        }
        copy.append(&top);
        copy.append(&muted(&issue.detail));
        let logos = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        for agent in issue
            .agents
            .iter()
            .filter_map(|id| inv.agents.iter().find(|a| &a.id == id))
        {
            let chip = gtk::CenterBox::new();
            chip.add_css_class("toolport-skill-chip");
            chip.add_css_class("logo");
            chip.set_size_request(24, 24);
            chip.set_tooltip_text(Some(&agent.name));
            chip.update_property(&[gtk::accessible::Property::Label(&agent.name)]);
            chip.set_center_widget(Some(&logo(&agent.id, 14)));
            logos.append(&chip);
        }
        if logos.first_child().is_some() {
            copy.append(&logos);
        }
        let chips = gtk::FlowBox::builder()
            .selection_mode(gtk::SelectionMode::None)
            .column_spacing(6)
            .row_spacing(6)
            .max_children_per_line(40)
            .homogeneous(false)
            .build();
        for name in issue.skill_names.iter().take(ISSUE_SKILL_LIMIT) {
            let line = gtk::Box::new(gtk::Orientation::Horizontal, 4);
            let star = gtk::Image::from_icon_name("starred-symbolic");
            star.set_pixel_size(12);
            star.add_css_class("toolport-muted");
            line.append(&star);
            let label = gtk::Label::new(Some(name));
            label.add_css_class("caption");
            line.append(&label);
            // A chip opens that skill on the Skills tab, where it can be switched or removed.
            let chip = gtk::Button::builder()
                .child(&line)
                .halign(gtk::Align::Start)
                .css_classes(["flat", "toolport-skill-chip"])
                .tooltip_text(crate::i18n::trf("Show {skill}", &[("skill", name)]))
                .build();
            let page = self.clone();
            let keys = issue.skills.clone();
            let name = name.clone();
            chip.connect_clicked(move |_| page.show_skill(&keys, &name));
            chips.insert(&chip, -1);
        }
        if issue.skill_names.len() > ISSUE_SKILL_LIMIT {
            let more = muted(&crate::i18n::trf(
                "+{count} more",
                &[(
                    "count",
                    &(issue.skill_names.len() - ISSUE_SKILL_LIMIT).to_string(),
                )],
            ));
            more.add_css_class("caption");
            chips.insert(&more, -1);
        }
        if chips.first_child().is_some() {
            copy.append(&chips);
        }
        card.append(&copy);
        card
    }

    /// Jump from a Needs a look chip to that skill on the Skills tab.
    fn show_skill(&self, keys: &[String], name: &str) {
        let found = self.inventory.borrow().as_ref().and_then(|inv| {
            inv.skills
                .iter()
                .find(|s| s.name == name && keys.contains(&s.key))
                .map(|s| (s.key.clone(), s.source))
        });
        let Some((key, source)) = found else {
            return;
        };
        self.search.set_text("");
        self.collapsed.borrow_mut().remove(&source_key(source));
        self.selected_skill.replace(Some(key));
        self.skills.detail_scroll.vadjustment().set_value(0.0);
        self.select_tab(Tab::Skills);
    }

    fn confirm_fix(&self, id: &str, fingerprint: &str, title: &str, label: &str) {
        let Some(parent) = self.app.active_window() else {
            return;
        };
        #[allow(deprecated)]
        let dialog = adw::MessageDialog::new(
            Some(&parent),
            Some(label),
            Some(&crate::i18n::tr(
                "Toolport backs up any config file before it changes it, and never deletes a skill folder.",
            )),
        );
        l10n::bind_dialog_body(
            &dialog,
            "Toolport backs up any config file before it changes it, and never deletes a skill folder.",
        );
        dialog.add_response("cancel", &crate::i18n::tr("Cancel"));
        l10n::bind_dialog_response(&dialog, "cancel", "Cancel");
        dialog.add_response("fix", label);
        dialog.set_close_response("cancel");
        dialog.set_default_response(Some("cancel"));
        dialog.set_response_appearance("fix", adw::ResponseAppearance::Suggested);
        let page = self.clone();
        let id = id.to_string();
        let fingerprint = fingerprint.to_string();
        let success = crate::i18n::trf("Fixed: {title}", &[("title", title)]);
        dialog.connect_response(None, move |dialog, response| {
            if response == "fix" {
                let id = id.clone();
                let fingerprint = fingerprint.clone();
                let failed = titled(crate::i18n::tr("Couldn't fix this"));
                page.mutate(success.clone(), move |project| {
                    crate::skills::fix_issue(&id, &fingerprint, project.as_deref()).map_err(failed)
                });
            }
            dialog.close();
        });
        dialog.present();
    }
}

fn tristate(on: bool) -> gtk::AccessibleTristate {
    if on {
        gtk::AccessibleTristate::True
    } else {
        gtk::AccessibleTristate::False
    }
}

/// An error under a line saying what failed, as the React view's toast shows them.
fn titled(title: String) -> impl FnOnce(String) -> String + Send + 'static {
    move |error| format!("{title}\n{error}")
}

/// Put a scroller back where it was once the rebuilt content has been measured.
fn restore_scroll(scroll: &gtk::ScrolledWindow, value: f64) {
    if value <= 0.0 {
        return;
    }
    let adjustment = scroll.vadjustment();
    adjustment.set_value(value);
    gtk::glib::idle_add_local_once(move || adjustment.set_value(value));
}

fn path_value(path: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(path)
        .halign(gtk::Align::Fill)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::Char)
        .selectable(true)
        .css_classes(["monospace"])
        .build()
}

fn value_label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .halign(gtk::Align::Fill)
        .xalign(0.0)
        .wrap(true)
        .wrap_mode(gtk::pango::WrapMode::WordChar)
        .build()
}

fn agent_summary(inv: &Inventory, agent: &Agent) -> gtk::Box {
    let row = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    row.add_css_class("toolport-skill-agent");
    row.append(&logo_tile(&agent.id));
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.append(&heading(&agent.name));
    let count = |state: State| {
        inv.skills
            .iter()
            .filter(|s| s.states.get(&agent.id).is_some_and(|x| x.state == state))
            .count()
    };
    let mut parts = Vec::new();
    if agent.supports_skills {
        parts.push(crate::i18n::trf(
            "{count} on",
            &[("count", &count(State::On).to_string())],
        ));
        let name_only = count(State::NameOnly);
        if name_only > 0 {
            parts.push(crate::i18n::trf(
                "{count} name only",
                &[("count", &name_only.to_string())],
            ));
        }
        let off = count(State::Off);
        if off > 0 {
            parts.push(crate::i18n::trf(
                "{count} off",
                &[("count", &off.to_string())],
            ));
        }
    } else {
        parts.push(crate::i18n::tr("No file-based skills"));
    }
    let rules = inv
        .rules
        .iter()
        .filter(|r| r.agents.contains_key(&agent.id))
        .count();
    parts.push(match rules {
        0 => crate::i18n::tr("no rules files"),
        1 => crate::i18n::tr("1 rules file"),
        _ => crate::i18n::trf("{count} rules files", &[("count", &rules.to_string())]),
    });
    let summary = muted(&parts.join(" · "));
    summary.add_css_class("caption");
    copy.append(&summary);
    row.append(&copy);
    let broken = count(State::Broken);
    let status = if broken > 0 {
        badge(
            &crate::i18n::trf("{count} broken", &[("count", &broken.to_string())]),
            "error",
        )
    } else if agent.supports_skills {
        let label = badge(
            &if agent.can_toggle {
                crate::i18n::tr("Own switch")
            } else {
                crate::i18n::tr("No switch")
            },
            "",
        );
        label.set_tooltip_text(Some(&agent.switch_note));
        label
    } else {
        return row;
    };
    status.set_valign(gtk::Align::Center);
    row.append(&status);
    row
}
