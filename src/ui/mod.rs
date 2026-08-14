//! Main application window: lists imported OpenVPN profiles, allows
//! importing new `.ovpn` files, connecting/disconnecting, and shows the
//! current connection status.

use crate::nm::{NetworkManager, VpnProfile};
use crate::secrets;
use gtk::prelude::*;
use relm4::prelude::*;
use relm4::{ComponentParts, ComponentSender, RelmApp, RelmWidgetExt};

/// Top level application state.
pub struct App {
    profiles: Vec<VpnProfile>,
    status: String,
    busy: bool,
    input_sender: relm4::Sender<AppMsg>,
}

#[derive(Debug)]
pub enum AppMsg {
    /// Triggered on startup and after any action to refresh the profile
    /// list and active connection from NetworkManager.
    Refresh,
    /// Result of a refresh, carrying the fetched profile list.
    Refreshed(Vec<VpnProfile>),
    /// User picked a `.ovpn` file to import.
    Import(std::path::PathBuf),
    /// User asked to connect to the given profile name.
    Connect(String),
    /// User asked to disconnect the given profile name.
    Disconnect(String),
    /// User asked to delete the given profile name.
    Delete(String),
    /// Open the GTK file chooser to pick a `.ovpn` file.
    OpenImportDialog,
    /// User asked to edit the credentials/properties of an existing profile.
    OpenEditDialog { name: String, uuid: String },
    /// User submitted edited properties from the edit dialog. Each field is
    /// `Some` only if the user actually changed it from its pre-filled
    /// value, so unparsed/unknown original values are left untouched
    /// rather than being clobbered with a blank.
    SaveCredentials {
        name: String,
        uuid: String,
        username: Option<String>,
        password: Option<String>,
        remote: Option<String>,
        port: Option<String>,
        protocol_tcp: Option<bool>,
        cipher: Option<String>,
    },
    /// An async action reported an error to surface in the status label.
    Error(String),
}

#[relm4::component(pub)]
impl SimpleComponent for App {
    type Init = ();
    type Input = AppMsg;
    type Output = ();

    view! {
        gtk::Window {
            set_title: Some("OpenVPN Manager"),
            set_default_size: (480, 400),

            gtk::Box {
                set_orientation: gtk::Orientation::Vertical,
                set_spacing: 8,
                set_margin_all: 12,

                gtk::Box {
                    set_orientation: gtk::Orientation::Horizontal,
                    set_spacing: 8,

                    gtk::Label {
                        set_label: "OpenVPN Profiles",
                        set_hexpand: true,
                        set_halign: gtk::Align::Start,
                        add_css_class: "title-3",
                    },

                    gtk::Button {
                        set_label: "Import .ovpn",
                        connect_clicked => AppMsg::OpenImportDialog,
                    },

                    gtk::Button {
                        set_icon_name: "view-refresh-symbolic",
                        set_tooltip_text: Some("Refresh"),
                        connect_clicked => AppMsg::Refresh,
                    },
                },

                gtk::ScrolledWindow {
                    set_vexpand: true,
                    set_hscrollbar_policy: gtk::PolicyType::Never,

                    #[name = "profile_list"]
                    gtk::ListBox {
                        set_selection_mode: gtk::SelectionMode::None,
                        add_css_class: "boxed-list",
                    }
                },

                gtk::Label {
                    #[watch]
                    set_label: &model.status,
                    set_halign: gtk::Align::Start,
                    add_css_class: "dim-label",
                },
            }
        }
    }

    fn init(
        _init: Self::Init,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let model = App {
            profiles: Vec::new(),
            status: "Loading profiles...".to_string(),
            busy: false,
            input_sender: sender.input_sender().clone(),
        };

        let widgets = view_output!();

        sender.input(AppMsg::Refresh);

        ComponentParts { model, widgets }
    }

    fn update(&mut self, msg: Self::Input, sender: ComponentSender<Self>) {
        eprintln!("[ui] update: {msg:?}");
        match msg {
            AppMsg::Refresh => {
                self.status = "Loading profiles...".to_string();
                let sender = sender.clone();
                relm4::spawn(async move {
                    eprintln!("[ui] Refresh task started");
                    let nm = NetworkManager::new();
                    let msg = match nm.list_profiles().await {
                        Ok(profiles) => {
                            eprintln!("[ui] Refresh task got {} profiles", profiles.len());
                            AppMsg::Refreshed(profiles)
                        }
                        Err(err) => {
                            eprintln!("[ui] Refresh task failed: {err:#}");
                            AppMsg::Error(format!("Failed to list profiles: {err:#}"))
                        }
                    };
                    eprintln!("[ui] Refresh task sending {msg:?}");
                    sender.input(msg);
                });
            }
            AppMsg::Refreshed(profiles) => {
                let active = profiles.iter().find(|p| p.active).map(|p| p.name.clone());
                self.status = match active {
                    Some(name) => format!("Connected: {name}"),
                    None => format!("{} profile(s), disconnected", profiles.len()),
                };
                self.profiles = profiles;
                self.busy = false;
            }
            AppMsg::OpenImportDialog => {
                let sender = sender.clone();
                relm4::spawn_local(async move {
                    let dialog = gtk::FileChooserDialog::new(
                        Some("Import OpenVPN Profile"),
                        None::<&gtk::Window>,
                        gtk::FileChooserAction::Open,
                        &[
                            ("Cancel", gtk::ResponseType::Cancel),
                            ("Import", gtk::ResponseType::Accept),
                        ],
                    );
                    let filter = gtk::FileFilter::new();
                    filter.add_pattern("*.ovpn");
                    filter.set_name(Some("OpenVPN config (*.ovpn)"));
                    dialog.add_filter(&filter);

                    dialog.connect_response(move |dialog, response| {
                        if response == gtk::ResponseType::Accept {
                            if let Some(file) = dialog.file() {
                                if let Some(path) = file.path() {
                                    sender.input(AppMsg::Import(path));
                                }
                            }
                        }
                        dialog.close();
                    });
                    dialog.show();
                });
            }
            AppMsg::Import(path) => {
                self.status = "Importing profile...".to_string();
                self.busy = true;
                let sender = sender.clone();
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    match nm.import_profile(&path).await {
                        Ok(name) => {
                            let profiles = match nm.list_profiles().await {
                                Ok(profiles) => profiles,
                                Err(err) => {
                                    sender.input(AppMsg::Error(format!(
                                        "Failed to refresh: {err:#}"
                                    )));
                                    return;
                                }
                            };

                            let uuid = profiles
                                .iter()
                                .find(|p| p.name == name)
                                .map(|p| p.uuid.clone());

                            sender.input(AppMsg::Refreshed(profiles));

                            // If the imported profile needs a username/password,
                            // open the edit dialog automatically so the user
                            // isn't left with a profile that will fail to
                            // connect until they discover the Edit button.
                            if let Some(uuid) = uuid {
                                if let Ok(details) = nm.connection_details(&name).await {
                                    if details.needs_auth {
                                        sender.input(AppMsg::OpenEditDialog { name, uuid });
                                    }
                                }
                            }
                        }
                        Err(err) => {
                            sender.input(AppMsg::Error(format!("Import failed: {err:#}")));
                        }
                    }
                });
            }
            AppMsg::Connect(name) => {
                self.status = format!("Connecting to {name}...");
                self.busy = true;
                let sender = sender.clone();
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    let msg = match nm.connect(&name).await {
                        Err(err) => AppMsg::Error(format!("Connect failed: {err:#}")),
                        Ok(()) => match nm.list_profiles().await {
                            Ok(profiles) => AppMsg::Refreshed(profiles),
                            Err(err) => AppMsg::Error(format!("Failed to refresh: {err:#}")),
                        },
                    };
                    sender.input(msg);
                });
            }
            AppMsg::Disconnect(name) => {
                self.status = format!("Disconnecting {name}...");
                self.busy = true;
                let sender = sender.clone();
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    let msg = match nm.disconnect(&name).await {
                        Err(err) => AppMsg::Error(format!("Disconnect failed: {err:#}")),
                        Ok(()) => match nm.list_profiles().await {
                            Ok(profiles) => AppMsg::Refreshed(profiles),
                            Err(err) => AppMsg::Error(format!("Failed to refresh: {err:#}")),
                        },
                    };
                    sender.input(msg);
                });
            }
            AppMsg::Delete(name) => {
                self.busy = true;
                let sender = sender.clone();
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    let msg = match nm.delete_profile(&name).await {
                        Err(err) => AppMsg::Error(format!("Delete failed: {err:#}")),
                        Ok(()) => match nm.list_profiles().await {
                            Ok(profiles) => AppMsg::Refreshed(profiles),
                            Err(err) => AppMsg::Error(format!("Failed to refresh: {err:#}")),
                        },
                    };
                    sender.input(msg);
                });
            }
            AppMsg::OpenEditDialog { name, uuid } => {
                let sender = sender.clone();
                relm4::spawn_local(async move {
                    let nm = NetworkManager::new();
                    let details = nm.connection_details(&name).await.unwrap_or_default();

                    let dialog = gtk::Dialog::with_buttons(
                        Some(&format!("Edit connection: {name}")),
                        None::<&gtk::Window>,
                        gtk::DialogFlags::MODAL,
                        &[
                            ("Cancel", gtk::ResponseType::Cancel),
                            ("Save", gtk::ResponseType::Accept),
                        ],
                    );

                    let content = dialog.content_area();
                    content.set_orientation(gtk::Orientation::Vertical);
                    content.set_spacing(8);
                    content.set_margin_top(12);
                    content.set_margin_bottom(12);
                    content.set_margin_start(12);
                    content.set_margin_end(12);

                    let field_label = |text: &str| {
                        gtk::Label::builder()
                            .label(text)
                            .halign(gtk::Align::Start)
                            .build()
                    };

                    let username_entry = gtk::Entry::builder()
                        .placeholder_text("Username")
                        .text(details.username.as_deref().unwrap_or(""))
                        .build();
                    content.append(&field_label("Username"));
                    content.append(&username_entry);

                    let password_entry = gtk::PasswordEntry::builder()
                        .placeholder_text("Leave blank to keep current password")
                        .show_peek_icon(true)
                        .build();
                    content.append(&field_label("Password"));
                    content.append(&password_entry);

                    content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

                    let remote_entry = gtk::Entry::builder()
                        .placeholder_text("Server address")
                        .text(details.remote.as_deref().unwrap_or(""))
                        .build();
                    content.append(&field_label("Remote server"));
                    content.append(&remote_entry);

                    let port_entry = gtk::Entry::builder()
                        .placeholder_text("Port")
                        .text(details.port.as_deref().unwrap_or(""))
                        .build();
                    content.append(&field_label("Port"));
                    content.append(&port_entry);

                    let protocol_combo = gtk::ComboBoxText::new();
                    protocol_combo.append(Some("udp"), "UDP");
                    protocol_combo.append(Some("tcp"), "TCP");
                    protocol_combo
                        .set_active_id(Some(details.protocol.as_deref().unwrap_or("udp")));
                    content.append(&field_label("Protocol"));
                    content.append(&protocol_combo);

                    let cipher_entry = gtk::Entry::builder()
                        .placeholder_text("Cipher (e.g. AES-256-GCM)")
                        .text(details.cipher.as_deref().unwrap_or(""))
                        .build();
                    content.append(&field_label("Cipher"));
                    content.append(&cipher_entry);

                    let name_for_response = name.clone();
                    let uuid_for_response = uuid.clone();
                    let original = details.clone();
                    dialog.connect_response(move |dialog, response| {
                        if response == gtk::ResponseType::Accept {
                            // Only report fields that actually changed from
                            // their pre-filled value, so anything we failed
                            // to parse (and thus left blank) doesn't get
                            // clobbered on save.
                            let username = non_empty_if_changed(
                                username_entry.text().as_str(),
                                original.username.as_deref(),
                            );
                            let password = {
                                let text = password_entry.text();
                                if text.is_empty() {
                                    None
                                } else {
                                    Some(text.to_string())
                                }
                            };
                            let remote = non_empty_if_changed(
                                remote_entry.text().as_str(),
                                original.remote.as_deref(),
                            );
                            let port = non_empty_if_changed(
                                port_entry.text().as_str(),
                                original.port.as_deref(),
                            );
                            let cipher = non_empty_if_changed(
                                cipher_entry.text().as_str(),
                                original.cipher.as_deref(),
                            );
                            let protocol_tcp = protocol_combo.active_id().and_then(|id| {
                                let new_is_tcp = id == "tcp";
                                let original_is_tcp = original.protocol.as_deref() == Some("tcp");
                                (new_is_tcp != original_is_tcp).then_some(new_is_tcp)
                            });

                            sender.input(AppMsg::SaveCredentials {
                                name: name_for_response.clone(),
                                uuid: uuid_for_response.clone(),
                                username,
                                password,
                                remote,
                                port,
                                protocol_tcp,
                                cipher,
                            });
                        }
                        dialog.close();
                    });
                    dialog.show();
                });
            }
            AppMsg::SaveCredentials {
                name,
                uuid,
                username,
                password,
                remote,
                port,
                protocol_tcp,
                cipher,
            } => {
                self.status = "Saving connection settings...".to_string();
                self.busy = true;
                let sender = sender.clone();
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    let msg = async {
                        if let Some(username) = &username {
                            nm.set_username(&name, username).await?;
                        }
                        if let Some(remote) = &remote {
                            nm.set_remote(&name, remote).await?;
                        }
                        if let Some(port) = &port {
                            nm.set_port(&name, port).await?;
                        }
                        if let Some(cipher) = &cipher {
                            nm.set_cipher(&name, cipher).await?;
                        }
                        if let Some(is_tcp) = protocol_tcp {
                            nm.set_protocol_tcp(&name, is_tcp).await?;
                        }
                        if let Some(password) = &password {
                            nm.mark_password_agent_owned(&name).await?;
                            secrets::keyring::store_password(&uuid, &name, password)
                                .await
                                .map_err(|err| anyhow::anyhow!(err))?;
                        }
                        nm.list_profiles().await
                    }
                    .await;

                    let msg = match msg {
                        Ok(profiles) => AppMsg::Refreshed(profiles),
                        Err(err) => AppMsg::Error(format!("Failed to save connection: {err:#}")),
                    };
                    sender.input(msg);
                });
            }
            AppMsg::Error(message) => {
                self.status = message;
                self.busy = false;
            }
        }
    }

    fn post_view(&self) {
        while let Some(row) = profile_list.first_child() {
            profile_list.remove(&row);
        }

        for profile in &self.profiles {
            let row = gtk::Box::builder()
                .orientation(gtk::Orientation::Horizontal)
                .spacing(8)
                .margin_top(6)
                .margin_bottom(6)
                .margin_start(6)
                .margin_end(6)
                .build();

            let label = gtk::Label::builder()
                .label(&profile.name)
                .hexpand(true)
                .halign(gtk::Align::Start)
                .build();
            row.append(&label);

            let toggle = gtk::Button::builder()
                .label(if profile.active {
                    "Disconnect"
                } else {
                    "Connect"
                })
                .build();
            {
                let name = profile.name.clone();
                let active = profile.active;
                let sender = self.input_sender.clone();
                toggle.connect_clicked(move |_| {
                    let msg = if active {
                        AppMsg::Disconnect(name.clone())
                    } else {
                        AppMsg::Connect(name.clone())
                    };
                    sender.emit(msg);
                });
            }
            row.append(&toggle);

            let edit = gtk::Button::builder()
                .icon_name("document-edit-symbolic")
                .tooltip_text("Edit connection settings")
                .build();
            {
                let name = profile.name.clone();
                let uuid = profile.uuid.clone();
                let sender = self.input_sender.clone();
                edit.connect_clicked(move |_| {
                    sender.emit(AppMsg::OpenEditDialog {
                        name: name.clone(),
                        uuid: uuid.clone(),
                    });
                });
            }
            row.append(&edit);

            let delete = gtk::Button::builder()
                .icon_name("user-trash-symbolic")
                .tooltip_text("Delete profile")
                .build();
            {
                let name = profile.name.clone();
                let sender = self.input_sender.clone();
                delete.connect_clicked(move |_| {
                    sender.emit(AppMsg::Delete(name.clone()));
                });
            }
            row.append(&delete);

            profile_list.append(&row);
        }
    }
}

impl App {}

/// Returns `Some(new)` if `new` is non-empty and differs from `original`
/// (treating an absent/unparsed `original` as different from any non-empty
/// value), or `None` if there's nothing to change - used so fields we
/// couldn't parse from an existing connection (and thus left blank in the
/// dialog) aren't clobbered unless the user actually typed something.
fn non_empty_if_changed(new: &str, original: Option<&str>) -> Option<String> {
    if new.is_empty() {
        return None;
    }
    if original == Some(new) {
        return None;
    }
    Some(new.to_string())
}

/// Launch the GTK application. Blocks until the window is closed.
pub fn run() {
    let app = RelmApp::new("org.omarchy.OpenvpnManager");
    app.run::<App>(());
}
