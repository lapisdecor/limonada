//! The account panel: sign in, see who is signed in, sign out.
//!
//! A right-hand panel like Settings, so it is non-modal: the feed stays visible
//! and reloads as the session takes effect. The password is written only into
//! this form's memory and sent once; nothing here persists it.

use eframe::egui;

use crate::app::Limonada;

pub fn show(app: &mut Limonada, ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    let mut close = false;
    let mut sign_out = false;
    let mut submit = false;

    egui::Panel::right("limonada-auth")
        .resizable(true)
        .default_size(330.0)
        .min_size(280.0)
        .max_size(520.0)
        .show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Account").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("✕").on_hover_text("Close").clicked() {
                        close = true;
                    }
                });
            });
            ui.separator();
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // --- Signed in ----------------------------------------
                    if let Some((username, instance)) = app
                        .settings
                        .active_session()
                        .map(|session| (session.username.clone(), session.instance_url.clone()))
                    {
                        ui.label(egui::RichText::new(format!("Signed in as {username}")).strong());
                        ui.label(egui::RichText::new(instance).weak().small());
                        ui.add_space(8.0);
                        if ui.button("Sign out").clicked() {
                            sign_out = true;
                        }
                        ui.add_space(8.0);
                        ui.label(
                            egui::RichText::new(
                                "Signing out forgets the stored token. Voting, commenting and \
                                 the subscribed listing need a session; browsing does not.",
                            )
                            .weak()
                            .small(),
                        );
                        return;
                    }

                    // --- Demo mode ----------------------------------------
                    if app.client.transport().is_offline() {
                        ui.label(egui::RichText::new("Demo mode cannot sign in").strong());
                        ui.add_space(4.0);
                        ui.label(
                            egui::RichText::new(
                                "Demo mode serves captured responses from fixtures/ and refuses \
                                 every write, so there is no instance to authenticate against. \
                                 Turn off Demo mode in Settings to sign in to a real instance.",
                            )
                            .weak()
                            .small(),
                        );
                        return;
                    }

                    // --- The form -----------------------------------------
                    ui.label("Username or email");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.auth.username)
                            .hint_text("your handle")
                            .desired_width(f32::INFINITY),
                    );

                    ui.add_space(6.0);
                    ui.label("Password");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.auth.password)
                            .password(true)
                            .desired_width(f32::INFINITY),
                    );

                    ui.add_space(6.0);
                    ui.label("Two-factor code (only if enabled)");
                    ui.add(
                        egui::TextEdit::singleline(&mut app.auth.totp)
                            .hint_text("optional")
                            .desired_width(f32::INFINITY),
                    );

                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let busy = app.auth.is_busy();
                        let ready =
                            !app.auth.username.trim().is_empty() && !app.auth.password.is_empty();
                        if ui
                            .add_enabled(!busy && ready, egui::Button::new("Sign in"))
                            .clicked()
                        {
                            submit = true;
                        }
                        if busy {
                            ui.spinner();
                            ui.label(egui::RichText::new("signing in…").weak().small());
                        }
                    });

                    if let Some(message) = app.auth.phase.error() {
                        ui.add_space(6.0);
                        ui.label(
                            egui::RichText::new(message)
                                .small()
                                .color(egui::Color32::from_rgb(220, 80, 80)),
                        );
                    }

                    ui.add_space(10.0);
                    ui.label(
                        egui::RichText::new(
                            "The password is sent once, to the instance above, and is never \
                             stored. Only the token it returns is kept — in the settings file, \
                             readable by you alone.",
                        )
                        .weak()
                        .small(),
                    );
                });
        });

    if close {
        app.auth_open = false;
    }
    if sign_out {
        app.sign_out(&ctx);
    }
    if submit {
        // Both are the configured instance: `settings.instance_url` is what the
        // transport was built from, and `normalised_instance` is that same value
        // trimmed, so the token is filed under the instance that issues it.
        let instance = app.settings.normalised_instance();
        app.auth.submit(&ctx, &app.client, &instance);
    }
}
