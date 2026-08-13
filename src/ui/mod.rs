//! Main application window: lists imported OpenVPN profiles, allows
//! importing new `.ovpn` files, connecting/disconnecting, and shows the
//! current connection status.

use crate::nm::{NetworkManager, VpnProfile};
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
        match msg {
            AppMsg::Refresh => {
                self.status = "Loading profiles...".to_string();
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    match nm.list_profiles().await {
                        Ok(profiles) => AppMsg::Refreshed(profiles),
                        Err(err) => AppMsg::Error(format!("Failed to list profiles: {err:#}")),
                    }
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
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    match nm.import_profile(&path).await {
                        Ok(_name) => {
                            let nm = NetworkManager::new();
                            match nm.list_profiles().await {
                                Ok(profiles) => AppMsg::Refreshed(profiles),
                                Err(err) => AppMsg::Error(format!("Failed to refresh: {err:#}")),
                            }
                        }
                        Err(err) => AppMsg::Error(format!("Import failed: {err:#}")),
                    }
                });
            }
            AppMsg::Connect(name) => {
                self.status = format!("Connecting to {name}...");
                self.busy = true;
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    if let Err(err) = nm.connect(&name).await {
                        return AppMsg::Error(format!("Connect failed: {err:#}"));
                    }
                    match nm.list_profiles().await {
                        Ok(profiles) => AppMsg::Refreshed(profiles),
                        Err(err) => AppMsg::Error(format!("Failed to refresh: {err:#}")),
                    }
                });
            }
            AppMsg::Disconnect(name) => {
                self.status = format!("Disconnecting {name}...");
                self.busy = true;
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    if let Err(err) = nm.disconnect(&name).await {
                        return AppMsg::Error(format!("Disconnect failed: {err:#}"));
                    }
                    match nm.list_profiles().await {
                        Ok(profiles) => AppMsg::Refreshed(profiles),
                        Err(err) => AppMsg::Error(format!("Failed to refresh: {err:#}")),
                    }
                });
            }
            AppMsg::Delete(name) => {
                self.busy = true;
                relm4::spawn(async move {
                    let nm = NetworkManager::new();
                    if let Err(err) = nm.delete_profile(&name).await {
                        return AppMsg::Error(format!("Delete failed: {err:#}"));
                    }
                    match nm.list_profiles().await {
                        Ok(profiles) => AppMsg::Refreshed(profiles),
                        Err(err) => AppMsg::Error(format!("Failed to refresh: {err:#}")),
                    }
                });
            }
            AppMsg::Error(message) => {
                self.status = message;
                self.busy = false;
            }
        }

        // Rebuild the profile list rows. Doing this on every update keeps
        // the implementation simple; for larger lists a `FactoryVecDeque`
        // would be preferable.
        let _ = sender;
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

/// Launch the GTK application. Blocks until the window is closed.
pub fn run() {
    let app = RelmApp::new("org.omarchy.OpenvpnManager");
    app.run::<App>(());
}
