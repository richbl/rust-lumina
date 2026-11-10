//! The main GUI component for the Lumina application
//!
//! This is a single-window interface for configuring automatic display brightness using the Relm4
//! framework and `GTK4` with `LibAdwaita`. It handles user input, state management, and
//! asynchronous tasks for reading and applying brightness settings

use crate::messages::{Cmd, Msg};
use crate::model::{
    App, AutoAdjustState, DetectionState, InitData, READ_EVERY_TICKS, RECOMPUTE_EVERY_TICKS,
    ScheduleState, ValidationState, WorkState,
};
use crate::work::Mode;
use lumina::config::{INTERVAL_MAX_SECONDS, INTERVAL_MIN_SECONDS};
use lumina::engine::Applier;
use lumina::{APP_ID, APP_NAME, app_title};
use relm4::actions::{RelmAction, RelmActionGroup};
use relm4::adw;
use relm4::adw::prelude::*;
use relm4::gtk;
use relm4::prelude::*;
use std::time::Duration;

/// Issue tracker used by the "Report an issue" row
const ISSUE_URL: &str = "https://github.com/richbl/rust-lumina/issues/new?template=issue-report.md";

/// Icon for the Status view. The GNOME HIG requires every view to carry both an icon and a
/// title so the switcher remains legible even when only icons fit
const STATUS_ICON: &str = "display-brightness-symbolic";

/// Icon for the Settings view
const SETTINGS_ICON: &str = "preferences-system-symbolic";

/// Window width, in scale points, below which the switcher moves from the header bar to the
/// bottom window edge
const NARROW_WIDTH_SP: f64 = 480.0;

/// Application init dimensions
const APP_WIDTH: i32 = 600;
const APP_HEIGHT: i32 = 780;

relm4::new_action_group!(WindowActionGroup, "win");
relm4::new_stateless_action!(AboutAction, WindowActionGroup, "about");

/// A background task that sends a `Cmd::Tick` message to the main component every second. This
/// drives the auto-adjust loop and periodic brightness reads
async fn tick_loop(out: relm4::Sender<Cmd>) {
    let mut interval = relm4::tokio::time::interval(Duration::from_secs(1));
    interval.tick().await;

    while out.send(Cmd::Tick).is_ok() {
        interval.tick().await;
    }
}

/// Convert a `u64` value to a `f64` for use in a GTK adjustment, allowing for fractional values
fn interval_spin_value(value: u64) -> f64 {
    #[allow(clippy::cast_precision_loss)]
    {
        value as f64
    }
}

/// The `AboutDialog` definition
fn show_about(parent: &adw::ApplicationWindow) {
    let about = adw::AboutDialog::builder()
        .application_name(APP_NAME)
        .application_icon(APP_ID)
        .version(env!("CARGO_PKG_VERSION"))
        .comments("Lumina Project Sites on GitHub:")
        .issue_url(ISSUE_URL)
        .developers(vec!["Rich Bloch".to_owned()])
        .license_type(gtk::License::MitX11)
        .copyright("Copyright ©2026 Rich Bloch")
        .website("https://github.com/richbl/rust-lumina")
        .build();

    about.present(Some(parent));
}

/// Create the window actions, including the SIGINT key
fn register_window_actions(root: &adw::ApplicationWindow) {
    let about_action: RelmAction<AboutAction> = {
        let window = root.clone();
        RelmAction::new_stateless(move |_| show_about(&window))
    };

    let mut actions = RelmActionGroup::<WindowActionGroup>::new();
    actions.add_action(about_action);
    actions.register_for_widget(root);

    // Ctrl+Q, the standard GNOME shortcut for Quit
    relm4::main_application().set_accels_for_action("win.quit", &["<Control>q"]);
}

/// Configure the view switcher breakpoint, which is where "view switchers should switch to the
/// bottom window edge if the window becomes too narrow for them to fit in the header bar", in
/// accordance with the Gnome HIG
fn configure_breakpoint(
    root: &adw::ApplicationWindow,
    view_switcher: &adw::ViewSwitcher,
    view_switcher_bar: &adw::ViewSwitcherBar,
) {
    let narrow = adw::Breakpoint::new(adw::BreakpointCondition::new_length(
        adw::BreakpointConditionLengthType::MaxWidth,
        NARROW_WIDTH_SP,
        adw::LengthUnit::Sp,
    ));
    narrow.add_setter(view_switcher, "visible", Some(&false.to_value()));
    narrow.add_setter(view_switcher_bar, "reveal", Some(&true.to_value()));
    root.add_breakpoint(narrow);
}

/// Start the recurring one-second tick loop that drives the auto-adjust scheduler and periodic
/// brightness reads. The loop runs in the background and emits `Cmd::Tick` messages to the main
/// component so it can recompute state and apply scheduled brightness updates
fn start_tick_loop(sender: &ComponentSender<App>) {
    sender.command(|out, shutdown| shutdown.register(tick_loop(out)).drop_on_shutdown());
}

/// The main application component for Lumina, implementing the `Component` trait from Relm4,
/// managing the application state, handling user input, and coordinating asynchronous tasks for
/// reading and applying brightness settings
#[relm4::component(pub)]
impl Component for App {
    type Init = InitData;
    type Input = Msg;
    type Output = ();
    type CommandOutput = Cmd;

    view! {

        #[root]
        adw::ApplicationWindow {
            set_title: Some(&app_title()),
            set_default_size: (APP_WIDTH, APP_HEIGHT),

            #[wrap(Some)]
            set_content = &adw::ToolbarView {
                #[name = "view_stack"]
                #[wrap(Some)]
                set_content = &adw::ViewStack {

                    // ============================================================
                    // View 1: Status
                    // ============================================================
                    //
                    // App identity plus a quick readout of what Lumina is doing
                    add_titled[None, "Status"] = &adw::PreferencesPage {

                        // Hero header
                        add = &adw::PreferencesGroup {
                            gtk::Box {
                                set_orientation: gtk::Orientation::Vertical,
                                set_halign: gtk::Align::Center,

                                gtk::Image {
                                    set_pixel_size: 96,
                                    set_icon_name: Some(APP_ID),
                                },
                                gtk::Label {
                                    set_label: APP_NAME,
                                    add_css_class: "title-1",
                                },
                                gtk::Label {
                                    set_label: lumina::ABOUT,
                                    set_wrap: true,
                                    set_justify: gtk::Justification::Center,
                                    set_max_width_chars: 40,
                                    add_css_class: "dim-label",
                                },
                            },
                        },

                        // Brightness
                        add = &adw::PreferencesGroup {
                            set_title: "Brightness",
                            adw::ActionRow {
                                set_title: "Current",
                                set_use_markup: false,
                                #[watch]
                                set_subtitle: &model.current_text,
                            },
                            adw::ActionRow {
                                set_title: "Target",
                                set_use_markup: false,
                                #[watch]
                                set_subtitle: &model.target_text(),
                                #[watch]
                                set_sensitive: model.inputs_ok(),
                                #[name = "apply_button"]
                                add_suffix = &gtk::Button {
                                    set_label: "Apply",
                                    set_valign: gtk::Align::Center,
                                    connect_clicked => Msg::ApplyNow,
                                },

                            },
                            adw::SwitchRow {
                                set_title: "Update automatically",
                                set_subtitle: "Re-apply the schedule every update interval",
                                set_active: model.automation.enabled,
                                #[watch]
                                set_sensitive: model.inputs_ok(),
                                connect_active_notify[sender] => move |r| {
                                    sender.input(Msg::ToggleAuto(r.is_active()));
                                },
                            },
                        },
                        // Location & Time of Day
                        add = &adw::PreferencesGroup {
                            set_title: "Location &amp; Time of Day",
                            adw::ActionRow {
                                set_title: "Location",
                                set_use_markup: false,
                                #[watch]
                                set_subtitle: &model.location_text(),
                            },
                            adw::ActionRow {
                                set_title: "Day starts",
                                set_use_markup: false,
                                #[watch]
                                set_subtitle: &model.day_starts_text(),
                            },
                            adw::ActionRow {
                                set_title: "Day ends",
                                set_use_markup: false,
                                #[watch]
                                set_subtitle: &model.day_ends_text(),
                            },
                        },
                    } -> status_page: adw::ViewStackPage {
                        set_icon_name: Some(STATUS_ICON),
                    },

                    // ============================================================
                    // View 2: Settings
                    // ============================================================
                    //
                    // Everything that shapes how the schedule is computed and applied
                    add_titled[None, "Settings"] = &adw::PreferencesPage {

                        // Location
                        add = &adw::PreferencesGroup {
                            set_title: "Location",
                            adw::SwitchRow {
                                set_title: "Detect location automatically",
                                set_active: model.form.auto_location,
                                connect_active_notify[sender] => move |r| {
                                    sender.input(Msg::AutoLocation(r.is_active()));
                                },
                            },
                            adw::SpinRow {
                                set_title: "Latitude",
                                set_adjustment: Some(&gtk::Adjustment::new(
                                    model.form.latitude, -90.0, 90.0, 0.01, 1.0, 0.0,
                                )),
                                set_digits: 4,
                                #[watch]
                                set_visible: !model.form.auto_location,
                                #[watch]
                                #[block_signal(lat_changed)]
                                set_value: model.form.latitude,
                                connect_value_notify[sender] => move |r| {
                                    sender.input(Msg::Latitude(r.value()));
                                } @lat_changed,
                            },
                            adw::SpinRow {
                                set_title: "Longitude",
                                set_adjustment: Some(&gtk::Adjustment::new(
                                    model.form.longitude, -180.0, 180.0, 0.01, 1.0, 0.0,
                                )),
                                set_digits: 4,
                                #[watch]
                                set_visible: !model.form.auto_location,
                                #[watch]
                                #[block_signal(lon_changed)]
                                set_value: model.form.longitude,
                                connect_value_notify[sender] => move |r| {
                                    sender.input(Msg::Longitude(r.value()));
                                } @lon_changed,
                            },
                        },

                        // Brightness range
                        add = &adw::PreferencesGroup {
                            set_title: "Brightness Range",
                            // Both rows turn red when minimum exceeds maximum.
                            adw::SpinRow {
                                set_title: "Minimum (%)",
                                set_adjustment: Some(&gtk::Adjustment::new(
                                  f64::from(model.form.min), 0.0, 100.0, 1.0, 10.0, 0.0,
                                )),
                                #[watch]
                                set_class_active: ("error", model.validation.fields.range_invalid),
                                connect_value_notify[sender] => move |r| {
                                    sender.input(Msg::Min(r.value()));
                                },
                            },
                            adw::SpinRow {
                                set_title: "Maximum (%)",
                                set_adjustment: Some(&gtk::Adjustment::new(
                                    f64::from(model.form.max), 0.0, 100.0, 1.0, 10.0, 0.0,
                                )),
                                #[watch]
                                set_class_active: ("error", model.validation.fields.range_invalid),
                                connect_value_notify[sender] => move |r| {
                                    sender.input(Msg::Max(r.value()));
                                },
                            },
                        },

                        // Brightness schedule
                        add = &adw::PreferencesGroup {
                            set_title: "Brightness Schedule",
                            adw::SwitchRow {
                                set_title: "Follow sunrise/sunset automatically",
                                set_active: !model.schedule.custom_times,
                                connect_active_notify[sender] => move |r| {
                                    sender.input(Msg::FollowSun(r.is_active()));
                                },
                            },
                            adw::EntryRow {
                                set_title: "Day starts (HH:MM)",
                                #[watch]
                                set_visible: model.schedule.custom_times,
                                #[watch]
                                set_class_active: ("error", model.validation.fields.start_invalid),
                                #[track(model.schedule.sync_time_entries)]
                                #[block_signal(start_changed)]
                                set_text: &model.form.start,
                                connect_changed[sender] => move |e| {
                                    sender.input(Msg::Start(e.text().to_string()));
                                } @start_changed,
                            },
                            adw::EntryRow {
                                set_title: "Day ends (HH:MM)",
                                #[watch]
                                set_visible: model.schedule.custom_times,
                                #[watch]
                                set_class_active: ("error", model.validation.fields.stop_invalid),
                                #[track(model.schedule.sync_time_entries)]
                                #[block_signal(stop_changed)]
                                set_text: &model.form.stop,
                                connect_changed[sender] => move |e| {
                                    sender.input(Msg::Stop(e.text().to_string()));
                                } @stop_changed,
                            },
                            adw::SwitchRow {
                                set_title: "Respect manual changes",
                                set_subtitle: "Pause auto-updating if brightness is changed elsewhere",
                                set_active: model.form.respect_manual_changes,
                                connect_active_notify[sender] => move |r| {
                                    sender.input(Msg::RespectManual(r.is_active()));
                                },
                            },
                            adw::SpinRow {
                                set_title: "Update interval (seconds)",
                                set_adjustment: Some(&gtk::Adjustment::new(
                                    interval_spin_value(model.form.interval_seconds),
                                    interval_spin_value(INTERVAL_MIN_SECONDS),
                                    interval_spin_value(INTERVAL_MAX_SECONDS),
                                    1.0, 10.0, 0.0,
                                )),
                                #[watch]
                                set_sensitive: model.inputs_ok(),
                                connect_value_notify[sender] => move |r| {
                                    sender.input(Msg::Interval(r.value()));
                                },
                            },
                        },
                    } -> settings_page: adw::ViewStackPage {
                        set_icon_name: Some(SETTINGS_ICON),
                        #[watch]
                        set_needs_attention: model.validation.fields.range_invalid
                            || model.validation.fields.start_invalid
                            || model.validation.fields.stop_invalid,
                    },
                },

                add_top_bar = &adw::HeaderBar {
                    #[name = "view_switcher"]
                    #[wrap(Some)]
                    set_title_widget = &adw::ViewSwitcher {
                        set_stack: Some(&view_stack),
                        set_policy: adw::ViewSwitcherPolicy::Wide,
                    },

                    // Primary menu: end of the header bar, per the GNOME HIG
                    pack_end = &gtk::MenuButton {
                        set_icon_name: "open-menu-symbolic",
                        set_tooltip_text: Some("Main Menu"),
                        set_primary: true, // opens with F10
                        set_menu_model: Some(&primary_menu),
                    },
                },

                // Bottom bar, revealed when the window is too narrow for the header switcher
                #[name = "view_switcher_bar"]
                add_bottom_bar = &adw::ViewSwitcherBar {
                    set_stack: Some(&view_stack),
                },

            },
        }
    }

    menu! {
        primary_menu: {
            section! {
                &format!("About {APP_NAME}") => AboutAction,
            },
        }
    }

    /// Initialize the component with the given `InitData`, setting up the initial state and
    /// starting any necessary background tasks
    fn init(
        init: InitData,
        root: Self::Root,
        sender: ComponentSender<Self>,
    ) -> ComponentParts<Self> {
        let custom_times = !init.form.start.trim().is_empty() || !init.form.stop.trim().is_empty();
        let respect = init.form.respect_manual_changes;

        let mut model = App {
            form: init.form,
            config_path: init.config_path,
            config_checked: false,
            hold_save: init.load_failed,
            detected: None,
            detection: DetectionState::default(),
            automation: AutoAdjustState::default(),
            ticks: 0,
            applier: Applier::default().respecting_manual_changes(respect),
            job: WorkState::default(),
            current_text: "Reading…".into(),
            schedule: ScheduleState {
                custom_times,
                saved_start: String::new(),
                saved_stop: String::new(),
                sync_time_entries: false,
            },
            target: None,
            solar: None,
            solar_text: "—".into(),
            validation: ValidationState::default(),
        };

        if model.form.auto_location {
            model.detect(&sender);
        }

        model.recompute();

        if model.schedule.custom_times {
            model.fill_blank_times();
            model.recompute();
        }

        model.work(&sender, Mode::Read);

        if let Some(display) = gtk::gdk::Display::default() {
            let path = format!("/{}/icons", APP_ID.replace('.', "/"));
            gtk::IconTheme::for_display(&display).add_resource_path(&path);
        }

        let widgets = view_output!();
        register_window_actions(&root);
        configure_breakpoint(&root, &widgets.view_switcher, &widgets.view_switcher_bar);
        start_tick_loop(&sender);

        ComponentParts { model, widgets }
    }

    /// Handle a [`Msg`] produced by user input or other events, updating the component's state and
    /// triggering any necessary side effects
    fn update(&mut self, msg: Msg, sender: ComponentSender<Self>, _root: &Self::Root) {
        self.schedule.sync_time_entries = false;

        let (settings_changed, run) = self.handle_input(msg, &sender);

        self.recompute();
        if let Some(mode) = run {
            self.work(&sender, mode);
        }
        if self.automation.enabled && settings_changed {
            self.automation.dirty = true;
        }

        if settings_changed {
            self.hold_save = false;
        }

        self.auto_save();
    }

    /// Handle a [`Cmd`] produced by background tasks, updating the component's state and
    /// triggering any necessary side effects
    fn update_cmd(&mut self, msg: Cmd, sender: ComponentSender<Self>, _root: &Self::Root) {
        self.schedule.sync_time_entries = false;
        match msg {
            Cmd::Tick => {
                self.ticks += 1;

                if self.ticks.is_multiple_of(RECOMPUTE_EVERY_TICKS) {
                    self.recompute();
                }

                if self.automation.enabled {
                    self.automation.elapsed = self.automation.elapsed.saturating_add(1);

                    if self.automation.dirty
                        || self.automation.elapsed >= self.form.interval_seconds
                    {
                        self.automation.dirty = false;
                        self.automation.elapsed = 0;
                        self.work(&sender, Mode::Auto);
                    }
                } else if self.ticks.is_multiple_of(READ_EVERY_TICKS) {
                    self.work(&sender, Mode::Read);
                }
            }
            Cmd::Detected(r) => {
                self.detection.in_flight = false;
                match r {
                    Ok((l, source)) => {
                        self.detected = Some(l);
                        self.detection.source = Some(source);
                    }
                    Err(e) => self.detection.error = Some(e),
                }
                self.recompute();
                self.auto_save();
                if self.automation.enabled {
                    self.automation.dirty = true;
                }
            }
            Cmd::Worked(generation, result) => self.finish_work(generation, result, &sender),
        }
    }
}
