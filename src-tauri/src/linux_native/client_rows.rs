//! Rows on the Clients page: one line per client with its controls on the right, and
//! Claude Code profiles grouped under one entry when a machine has several.

use super::*;

/// How a row is drawn: a full-width line, a tile in the two-column Available grid, or one
/// profile inside the Claude Code group.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum RowStyle {
    Line,
    Tile,
    Profile,
}

/// Minimum tile width: two tiles share a line only when both get at least this much.
const TILE_WIDTH: i32 = 330;

/// The short discovery summary for a connected client, and the full sentence for its tooltip.
fn discovery_summary(client: &state::ClientView) -> (String, String) {
    let capabilities = crate::clients::discovery_capabilities(&client.id);
    let chosen = client
        .discovery_mode
        .as_deref()
        .or(client.inherited_discovery.as_deref());
    if let Some(max) = capabilities.auto_full_max_tools_for(chosen) {
        // Auto lists a small catalog in full for this client, then switches to search.
        let text = crate::i18n::trf(
            "Auto: {mode}",
            &[(
                "mode",
                &crate::i18n::trf(
                    "full up to {count} tools, then lazy",
                    &[("count", &max.to_string())],
                ),
            )],
        );
        return (text.clone(), text);
    }
    match capabilities.resolve_mode(chosen) {
        "full" => (
            crate::i18n::tr("Full tool list"),
            crate::i18n::tr("Full tool list. Client per-tool permission rules need Full mode."),
        ),
        "grouped" => (
            crate::i18n::tr("Browse a server, then call tools"),
            crate::i18n::tr(
                "Browse a server, then call tools. Client per-tool permission rules need Full mode.",
            ),
        ),
        _ => (
            crate::i18n::tr("Search, then call tools"),
            crate::i18n::tr(
                "Search, then call tools. Client per-tool permission rules need Full mode.",
            ),
        ),
    }
}

/// "just now", "5m ago": the same words the React Clients page uses mid-sentence.
fn ago(timestamp_ms: u64) -> String {
    let seconds = epoch_ms().saturating_sub(timestamp_ms) / 1000;
    if seconds < 60 {
        return crate::i18n::tr("just now");
    }
    let count = if seconds < 3_600 {
        seconds / 60
    } else if seconds < 86_400 {
        seconds / 3_600
    } else {
        seconds / 86_400
    };
    crate::i18n::trf(
        if seconds < 3_600 {
            crate::i18n::msg("{count}m ago")
        } else if seconds < 86_400 {
            crate::i18n::msg("{count}h ago")
        } else {
            crate::i18n::msg("{count}d ago")
        },
        &[("count", &count.to_string())],
    )
}

/// The one-line detail under a client's name, and the fuller text for its tooltip.
fn client_detail(client: &state::ClientView, style: RowStyle) -> (String, String) {
    if client.config_error {
        let text = crate::i18n::tr("Configuration could not be read safely");
        return (text.clone(), text);
    }
    let connected = client.gateway_state == state::ClientGatewayState::Connected;
    let mut parts = Vec::new();
    let mut tooltip = Vec::new();
    if style == RowStyle::Profile {
        if let Some(profile) = &client.claude_profile {
            // A profile without a known command is already named by its folder.
            if profile.command.is_some() {
                parts.push(profile.folder.clone());
            }
            if let Some(ms) = profile.last_used_ms {
                parts.push(crate::i18n::trf(
                    "Last active {value0}",
                    &[("value0", &ago(ms))],
                ));
            }
        }
    }
    if connected {
        if !client.uses_connectors && client.other_server_count > 0 {
            parts.push(crate::i18n::trf(
                if client.other_server_count == 1 {
                    crate::i18n::msg("{value0} other server")
                } else {
                    crate::i18n::msg("{value0} other servers")
                },
                &[("value0", &client.other_server_count.to_string())],
            ));
        }
    } else if client.uses_connectors {
        parts.push(crate::i18n::tr("Uses account connectors"));
    } else if style != RowStyle::Profile {
        parts.push(if client.server_count == 0 {
            crate::i18n::tr("No local MCP servers")
        } else {
            crate::i18n::trf(
                if client.server_count == 1 {
                    crate::i18n::msg("{value0} local MCP server")
                } else {
                    crate::i18n::msg("{value0} local MCP servers")
                },
                &[("value0", &client.server_count.to_string())],
            )
        });
    }
    if client.movable_server_count > 0 && !client.uses_connectors {
        parts.push(crate::i18n::trf(
            "{value0} to import",
            &[("value0", &client.movable_server_count.to_string())],
        ));
    }
    if connected {
        if client.shared_http {
            parts.push("Shared HTTP".into());
        }
        let (short, full) = discovery_summary(client);
        parts.push(short);
        tooltip.push(full);
    }
    let text = parts.join(" · ");
    let tooltip = if tooltip.is_empty() {
        text.clone()
    } else {
        format!("{text}\n{}", tooltip.join("\n"))
    };
    (text, tooltip)
}

/// "Connected" or "Customized"; an unconnected row's Connect button already says it.
fn status_badge(client: &state::ClientView) -> gtk::Label {
    let connected = client.gateway_state == state::ClientGatewayState::Connected;
    let badge = l10n::label(if connected {
        crate::i18n::msg("Connected")
    } else {
        crate::i18n::msg("Customized")
    });
    let class = if connected { "success" } else { "review" };
    badge.add_css_class("toolport-badge");
    badge.add_css_class("caption");
    badge.add_css_class(class);
    badge.set_valign(gtk::Align::Center);
    badge
}

/// One client: name and a one-line detail on the left, its controls on the right.
pub(super) fn client_row(
    client: &state::ClientView,
    page: ClientPage,
    style: RowStyle,
) -> gtk::Box {
    let card = gtk::Box::new(gtk::Orientation::Vertical, 8);
    card.add_css_class(match style {
        RowStyle::Tile => "toolport-client-tile",
        _ => "toolport-card",
    });
    card.add_css_class("toolport-client-row");
    if style == RowStyle::Tile {
        card.set_size_request(TILE_WIDTH, -1);
    }
    if style == RowStyle::Profile {
        card.add_css_class("toolport-client-profile");
    }
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    card.append(&line);
    if style != RowStyle::Profile {
        let icon = branding::client_logo(&client.id);
        icon.set_valign(gtk::Align::Center);
        line.append(&icon);
    }
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let profile = client
        .claude_profile
        .as_ref()
        .filter(|_| style == RowStyle::Profile);
    // An unreadable profile has no account to show, signed in or not.
    let account_badge = profile.filter(|_| !client.config_error);
    let name = gtk::Label::builder()
        .label(match profile {
            Some(profile) => profile
                .command
                .clone()
                .unwrap_or_else(|| profile.folder.clone()),
            None => client.name.clone(),
        })
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .tooltip_text(crate::i18n::trf(
            "{value0}\nClient id: {value1}",
            &[("value0", &client.name), ("value1", &client.id)],
        ))
        .css_classes(["heading"])
        .build();
    if profile.is_some() {
        name.add_css_class("monospace");
    }
    title.append(&name);
    if let Some(profile) = account_badge {
        let account = match &profile.account {
            Some(account) => gtk::Label::new(Some(account)),
            None => l10n::label("No claude.ai sign-in"),
        };
        account.add_css_class("toolport-badge");
        account.add_css_class("caption");
        account.set_ellipsize(gtk::pango::EllipsizeMode::Middle);
        account.set_valign(gtk::Align::Center);
        title.append(&account);
    }
    copy.append(&title);
    let detail = gtk::Label::builder()
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .single_line_mode(true)
        .css_classes(["toolport-muted", "caption"])
        .build();
    {
        let client = client.clone();
        l10n::bind_label_with(&detail, move || client_detail(&client, style).0);
    }
    {
        let client = client.clone();
        l10n::bind_tooltip_with(&detail, move || client_detail(&client, style).1);
    }
    copy.append(&detail);
    line.append(&copy);

    let controls = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    controls.add_css_class("toolport-client-controls");
    controls.set_valign(gtk::Align::Center);
    controls.set_halign(gtk::Align::End);
    // In the Claude Code group, connected and unconnected profiles sit together; the
    // Connect button already says which ones are not connected.
    if (style == RowStyle::Profile && client.gateway_state == state::ClientGatewayState::Connected)
        || client.gateway_state == state::ClientGatewayState::Customized
    {
        controls.append(&status_badge(client));
    }
    if client.legacy_bearer_argv {
        let migrate = l10n::button("Review migration to stdio");
        let reviewed_client = client.clone();
        let reviewed_page = page.clone();
        migrate.connect_clicked(move |button| {
            confirm_client_reset(&reviewed_client, button.clone(), reviewed_page.clone());
        });
        controls.append(&migrate);
    }
    match client.gateway_state {
        state::ClientGatewayState::Disconnected => {
            let connect = l10n::button("Connect");
            connect.add_css_class("suggested-action");
            connect.set_sensitive(!client.config_error);
            let client_for_connect = client.clone();
            let page_for_connect = page.clone();
            connect.connect_clicked(move |_| {
                confirm_client_migrate(&client_for_connect, page_for_connect.clone());
            });
            controls.append(&connect);
        }
        state::ClientGatewayState::Customized if client_needs_reset_action(client) => {
            let reset = l10n::button("Reset");
            reset.add_css_class("toolport-secondary-action");
            let client_for_reset = client.clone();
            let page_for_reset = page.clone();
            reset.connect_clicked(move |button| {
                confirm_client_reset(&client_for_reset, button.clone(), page_for_reset.clone());
            });
            controls.append(&reset);
        }
        state::ClientGatewayState::Customized => {}
        state::ClientGatewayState::Connected => {
            controls.append(&client_scope_menu(client.clone(), page.clone()));
            controls.append(&connected_client_actions_menu(client.clone(), page.clone()));
        }
    }
    if client.location_removable && client.gateway_state == state::ClientGatewayState::Disconnected
    {
        controls.append(&remove_location_button(client, page.clone()));
    }
    if controls.first_child().is_some() {
        line.append(&controls);
    }
    if client.legacy_bearer_argv {
        let warning = l10n::label("This older HTTP connection exposes a bearer credential in process arguments. Review migration to stdio; it stays unchanged until you confirm.");
        warning.set_wrap(true);
        warning.set_xalign(0.0);
        warning.add_css_class("toolport-muted");
        card.append(&warning);
    }
    card
}

fn remove_location_button(client: &state::ClientView, page: ClientPage) -> gtk::Button {
    let remove = gtk::Button::from_icon_name("user-trash-symbolic");
    remove.add_css_class("flat");
    remove.set_valign(gtk::Align::Center);
    remove.set_sensitive(!client.config_error);
    let label = crate::i18n::trf("Remove location {value0}", &[("value0", &client.name)]);
    remove.set_tooltip_text(Some(&crate::i18n::tr("Remove location")));
    l10n::bind_tooltip(&remove, "Remove location");
    remove.update_property(&[gtk::accessible::Property::Label(&label)]);
    let id = client.id.clone();
    remove.connect_clicked(move |_| {
        let Some(parent) = page.root.root().and_downcast::<gtk::Window>() else {
            return;
        };
        #[allow(deprecated)]
        let dialog = adw::MessageDialog::new(
            Some(&parent),
            Some(&crate::i18n::tr("Remove client location?")),
            Some(&crate::i18n::tr("This forgets the added location. Your client config and backups are kept. Auto-detected Claude folders still appear in Clients.")),
        );
        l10n::bind_dialog_heading(&dialog, "Remove client location?");
        l10n::bind_dialog_body(&dialog, "This forgets the added location. Your client config and backups are kept. Auto-detected Claude folders still appear in Clients.");
        dialog.add_responses(&[("cancel", "Cancel"), ("remove", "Remove location")]);
        l10n::bind_dialog_response(&dialog, "cancel", "Cancel");
        l10n::bind_dialog_response(&dialog, "remove", "Remove location");
        dialog.set_close_response("cancel");
        dialog.set_default_response(Some("cancel"));
        dialog.set_response_appearance("remove", adw::ResponseAppearance::Destructive);
        let id = id.clone();
        let page = page.clone();
        dialog.connect_response(Some("remove"), move |_, _| {
            let id = id.clone();
            let page = page.clone();
            gtk::glib::spawn_future_local(async move {
                match gtk::gio::spawn_blocking(move || crate::clients::remove_client_location(&id))
                    .await
                {
                    Ok(Ok(())) => page
                        .refresh_with_confirmation(crate::i18n::tr("Client location removed.")),
                    Ok(Err(error)) => page.show_error(&error),
                    Err(_) => page.show_error(&crate::i18n::tr("Could not remove client location.")),
                }
            });
        });
        dialog.present();
    });
    remove
}

/// Claude Code's profiles under one entry. Copies of the default profile (same account in
/// `~/.claude`, read only through CLAUDE_CONFIG_DIR) sit behind a note instead of a row.
pub(super) fn claude_group(profiles: &[&state::ClientView], page: ClientPage) -> gtk::Box {
    let group = gtk::Box::new(gtk::Orientation::Vertical, 0);
    group.add_css_class("toolport-client-group");
    let (copies, mut shown): (Vec<&state::ClientView>, Vec<&state::ClientView>) =
        profiles.iter().copied().partition(|client| {
            client
                .claude_profile
                .as_ref()
                .is_some_and(|profile| profile.same_as.is_some())
        });
    // Profiles started by a command first, by command; the rest by folder.
    shown.sort_by_key(|client| {
        let profile = client.claude_profile.as_ref();
        let command = profile.and_then(|profile| profile.command.clone());
        (
            command.is_none(),
            command.unwrap_or_else(|| profile.map(|p| p.folder.clone()).unwrap_or_default()),
        )
    });
    if let [only] = shown.as_slice() {
        // One real profile and its copies: an ordinary row, with the note under it.
        group.append(&client_row(only, page.clone(), RowStyle::Line));
        if !copies.is_empty() {
            group.append(&copies_note(&copies, &shown, page));
        }
        return group;
    }
    let connected = shown
        .iter()
        .filter(|client| client.gateway_state == state::ClientGatewayState::Connected)
        .count();

    let header = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    let icon = branding::client_logo("claude-code");
    icon.set_valign(gtk::Align::Center);
    header.append(&icon);
    let copy = gtk::Box::new(gtk::Orientation::Vertical, 2);
    copy.set_hexpand(true);
    copy.set_valign(gtk::Align::Center);
    let title = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    // A product name, not translated.
    let name = gtk::Label::new(Some("Claude Code"));
    name.add_css_class("heading");
    name.set_xalign(0.0);
    title.append(&name);
    let count = l10n::formatted_label(
        if shown.len() == 1 {
            crate::i18n::msg("{value0} profile")
        } else {
            crate::i18n::msg("{value0} profiles")
        },
        &[("value0", &shown.len().to_string())],
    );
    count.add_css_class("toolport-badge");
    count.add_css_class("caption");
    count.set_valign(gtk::Align::Center);
    title.append(&count);
    copy.append(&title);
    let detail = l10n::formatted_label(
        "{connected} of {total} connected",
        &[
            ("connected", &connected.to_string()),
            ("total", &shown.len().to_string()),
        ],
    );
    detail.set_xalign(0.0);
    detail.add_css_class("toolport-muted");
    detail.add_css_class("caption");
    copy.append(&detail);
    header.append(&copy);
    let chevron = gtk::Image::from_icon_name(if page.claude_open.get() {
        "pan-down-symbolic"
    } else {
        "pan-end-symbolic"
    });
    chevron.add_css_class("toolport-muted");
    chevron.set_valign(gtk::Align::Center);
    header.append(&chevron);
    let toggle = gtk::Button::builder()
        .child(&header)
        .css_classes(["flat", "toolport-client-group-toggle"])
        .build();
    toggle.update_property(&[
        gtk::accessible::Property::Label(&crate::i18n::tr("Claude Code profiles")),
        gtk::accessible::Property::Description(&detail.text()),
    ]);
    toggle.update_state(&[gtk::accessible::State::Expanded(Some(
        page.claude_open.get(),
    ))]);
    let header_card = gtk::Box::new(gtk::Orientation::Vertical, 0);
    header_card.add_css_class("toolport-card");
    header_card.add_css_class("toolport-client-row");
    header_card.append(&toggle);
    group.append(&header_card);

    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for client in &shown {
        rows.append(&client_row(client, page.clone(), RowStyle::Profile));
    }
    if !copies.is_empty() {
        rows.append(&copies_note(&copies, &shown, page.clone()));
    }
    let revealer = gtk::Revealer::builder()
        .child(&rows)
        .reveal_child(page.claude_open.get())
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(150)
        .build();
    group.append(&revealer);
    {
        let page = page.clone();
        toggle.connect_clicked(move |toggle| {
            let open = !page.claude_open.get();
            page.claude_open.set(open);
            revealer.set_reveal_child(open);
            chevron.set_icon_name(Some(if open {
                "pan-down-symbolic"
            } else {
                "pan-end-symbolic"
            }));
            toggle.update_state(&[gtk::accessible::State::Expanded(Some(open))]);
        });
    }
    group
}

/// "Also found ~/.claude/.claude.json: the same account as claude …", with a toggle that
/// shows those copies as rows.
fn copies_note(
    copies: &[&state::ClientView],
    shown: &[&state::ClientView],
    page: ClientPage,
) -> gtk::Box {
    let note = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let line = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    line.add_css_class("toolport-card");
    line.add_css_class("toolport-client-profile");
    let folders = copies
        .iter()
        .filter_map(|client| client.claude_profile.as_ref())
        .map(|profile| profile.folder.clone())
        .collect::<Vec<_>>()
        .join(", ");
    let of = copies
        .iter()
        .filter_map(|client| client.claude_profile.as_ref()?.same_as.as_deref())
        .find_map(|id| shown.iter().find(|client| client.id == id))
        .and_then(|client| client.claude_profile.as_ref()?.command.clone())
        .unwrap_or_else(|| "claude".into());
    let text = l10n::formatted_label(
        "Also found {folder}: the same account as {command}. Claude Code only reads it when CLAUDE_CONFIG_DIR points there, as some background tools do.",
        &[("folder", &folders), ("command", &of)],
    );
    text.set_wrap(true);
    text.set_xalign(0.0);
    text.set_hexpand(true);
    text.add_css_class("toolport-muted");
    text.add_css_class("caption");
    line.append(&text);
    let show = page.claude_copies_shown.get();
    let toggle = if show {
        l10n::button("Hide")
    } else {
        l10n::button("Show")
    };
    toggle.add_css_class("flat");
    toggle.set_valign(gtk::Align::Center);
    line.append(&toggle);
    note.append(&line);
    let rows = gtk::Box::new(gtk::Orientation::Vertical, 0);
    for client in copies {
        rows.append(&client_row(client, page.clone(), RowStyle::Profile));
    }
    let revealer = gtk::Revealer::builder()
        .child(&rows)
        .reveal_child(show)
        .transition_type(gtk::RevealerTransitionType::SlideDown)
        .transition_duration(150)
        .build();
    note.append(&revealer);
    toggle.connect_clicked(move |toggle| {
        let show = !page.claude_copies_shown.get();
        page.claude_copies_shown.set(show);
        revealer.set_reveal_child(show);
        l10n::bind_button(toggle, if show { "Hide" } else { "Show" });
    });
    note
}

/// The Available grid: two tiles per line when there is room, one otherwise.
pub(super) fn available_grid(clients: &[&state::ClientView], page: &ClientPage) -> gtk::FlowBox {
    let grid = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .homogeneous(true)
        .min_children_per_line(1)
        .max_children_per_line(2)
        .column_spacing(10)
        .row_spacing(10)
        .build();
    grid.add_css_class("toolport-client-grid");
    for client in clients {
        let child = gtk::FlowBoxChild::new();
        // Keyboard focus goes straight to the tile's buttons.
        child.set_focusable(false);
        child.set_child(Some(&client_row(client, page.clone(), RowStyle::Tile)));
        grid.insert(&child, -1);
    }
    grid
}
