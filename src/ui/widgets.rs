//! Small shared pieces of the UI.

use eframe::egui;

use crate::time;

/// Score chip, e.g. `▲ 42`.
///
/// For contexts where the item cannot be voted on from here (a search result,
/// say). Where it can, [`vote_buttons`] draws the score itself.
pub fn score(ui: &mut egui::Ui, value: i64) {
    ui.label(egui::RichText::new(format!("▲ {value}")).weak());
}

/// What a click on the vote controls asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VoteAction {
    #[default]
    Nothing,
    /// Send this score: `1` upvote, `-1` downvote, `0` to take a vote back.
    Set(i16),
    /// The user asked to vote, but nobody is signed in.
    SignInRequired,
}

/// The score a click on `arrow` (`1` or `-1`) sends, given the vote already
/// recorded.
///
/// Clicking the arrow you already voted with removes the vote — Lemmy's `score`
/// of `0` — which is the only way to undo one, and what Lemmy's own interface
/// does. Split out from the widget so the rule is testable without a frame.
fn next_score(my_vote: Option<i16>, arrow: i16) -> i16 {
    if my_vote == Some(arrow) { 0 } else { arrow }
}

/// Up/down vote controls with the running score between them.
///
/// The controls are drawn whether or not anyone is signed in, so the layout
/// does not shift with the session. When signed out a click reports
/// [`VoteAction::SignInRequired`] instead of sending something the instance
/// would only reject.
pub fn vote_buttons(
    ui: &mut egui::Ui,
    score: i64,
    my_vote: Option<i16>,
    signed_in: bool,
) -> VoteAction {
    let up = my_vote == Some(1);
    let down = my_vote == Some(-1);

    let arrow = |ui: &mut egui::Ui, selected: bool, glyph: &str, hint: &str, value: i16| {
        if ui.selectable_label(selected, glyph).on_hover_text(hint).clicked() {
            return if signed_in {
                VoteAction::Set(next_score(my_vote, value))
            } else {
                VoteAction::SignInRequired
            };
        }
        VoteAction::Nothing
    };

    let up_action = arrow(
        ui,
        up,
        "▲",
        if up { "Remove your upvote" } else { "Upvote" },
        1,
    );
    ui.label(egui::RichText::new(score.to_string()).strong());
    let down_action = arrow(
        ui,
        down,
        "▼",
        if down { "Remove your downvote" } else { "Downvote" },
        -1,
    );

    match (up_action, down_action) {
        (VoteAction::Nothing, other) => other,
        (other, _) => other,
    }
}

/// Comment-count chip, e.g. `💬 12`.
pub fn comment_count(ui: &mut egui::Ui, value: i64) {
    ui.label(egui::RichText::new(format!("💬 {value}")).weak());
}

/// A "nsfw" marker, only shown when relevant.
pub fn nsfw_badge(ui: &mut egui::Ui, nsfw: bool) {
    if nsfw {
        ui.label(
            egui::RichText::new("NSFW")
                .small()
                .strong()
                .color(egui::Color32::from_rgb(220, 80, 80)),
        );
    }
}

/// Age text for a timestamp, or `"unknown"` when it cannot be parsed.
pub fn age(published: Option<&str>, now: i64) -> String {
    published
        .and_then(|value| time::age_of(value, now))
        .unwrap_or_else(|| "unknown".to_string())
}

/// The handle for a community: `!name`, qualified with its instance when it is
/// not hosted here.
///
/// Search spans the whole federation this instance knows, where two communities
/// can share a name — `!linux` on lemmy.ml and on lemmy.world, or `!technology`
/// on a dozen instances — so the host is the only thing that tells them apart.
/// A local community keeps the short form it is written with.
pub fn community_handle(community: &crate::api::types::Community) -> String {
    let name = format!("!{}", community.name);
    if community.local {
        return name;
    }
    match community.actor_id.as_deref().and_then(host_of) {
        Some(host) => format!("{name}@{host}"),
        None => name,
    }
}

/// The host of a URL, without scheme or a leading `www.`.
pub fn host_of(url: &str) -> Option<String> {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(trimmed);
    let host = without_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    if host.is_empty() {
        return None;
    }
    let host = host.strip_prefix("www.").unwrap_or(host);
    Some(host.to_string())
}

/// Thousands separators for counts, e.g. `12,345`.
pub fn group_digits(value: i64) -> String {
    let digits = value.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, ch) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

/// Truncate to `max` characters, respecting char boundaries, with an ellipsis.
pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A consistent error block. Returns true when the user asked to retry.
pub fn error_block(
    ui: &mut egui::Ui,
    title: &str,
    message: &str,
    retry_label: Option<&str>,
    hint: Option<&str>,
) -> bool {
    let mut retry = false;
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(
            egui::RichText::new(title)
                .strong()
                .color(egui::Color32::from_rgb(220, 80, 80)),
        );
        ui.add_space(2.0);
        ui.label(message);
        if let Some(hint) = hint {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(hint).weak());
        }
        if let Some(label) = retry_label {
            ui.add_space(6.0);
            retry = ui.button(label).clicked();
        }
    });
    retry
}

/// A centred spinner with a caption, used for loading states.
pub fn loading(ui: &mut egui::Ui, caption: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(12.0);
        ui.spinner();
        ui.add_space(4.0);
        ui.label(egui::RichText::new(caption).weak());
    });
}

/// A muted placeholder for genuinely empty results.
pub fn empty(ui: &mut egui::Ui, caption: &str, detail: Option<&str>) {
    ui.vertical_centered(|ui| {
        ui.add_space(12.0);
        ui.label(egui::RichText::new(caption).strong());
        if let Some(detail) = detail {
            ui.add_space(2.0);
            ui.label(egui::RichText::new(detail).weak());
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_digits_for_readability() {
        assert_eq!(group_digits(0), "0");
        assert_eq!(group_digits(42), "42");
        assert_eq!(group_digits(1_000), "1,000");
        assert_eq!(group_digits(12_345_678), "12,345,678");
    }

    #[test]
    fn extracts_hosts_from_typical_post_urls() {
        assert_eq!(
            host_of("https://lemmy.ml/pictrs/image/abc.png").as_deref(),
            Some("lemmy.ml")
        );
        assert_eq!(
            host_of("http://www.example.com/a/b?c=d").as_deref(),
            Some("example.com")
        );
        assert_eq!(host_of("example.org").as_deref(), Some("example.org"));
        assert_eq!(host_of("https://example.org").as_deref(), Some("example.org"));
    }

    #[test]
    fn returns_none_for_urls_without_a_host() {
        assert_eq!(host_of(""), None);
        assert_eq!(host_of("   "), None);
        assert_eq!(host_of("https:///nohost"), None);
    }

    #[test]
    fn truncation_respects_char_boundaries() {
        assert_eq!(truncate("hello", 10), "hello");
        assert_eq!(truncate("hello", 5), "hello");
        assert_eq!(truncate("hello world", 6), "hello…");
        // Multi-byte input must not panic.
        assert_eq!(truncate("naïve café", 6), "naïve…");
    }

    #[test]
    fn voting_the_same_way_twice_takes_the_vote_back() {
        // Nothing voted: each arrow casts its own vote.
        assert_eq!(next_score(None, 1), 1);
        assert_eq!(next_score(None, -1), -1);
        // Clicking the arrow already chosen clears it, which is score 0.
        assert_eq!(next_score(Some(1), 1), 0);
        assert_eq!(next_score(Some(-1), -1), 0);
        // Clicking the other arrow switches the vote rather than clearing it.
        assert_eq!(next_score(Some(1), -1), -1);
        assert_eq!(next_score(Some(-1), 1), 1);
    }

    #[test]
    fn age_falls_back_when_the_timestamp_is_unusable() {
        assert_eq!(age(None, 0), "unknown");
        assert_eq!(age(Some("garbage"), 0), "unknown");
        assert_eq!(
            age(Some("2026-10-14T15:44:02Z"), 1_791_992_642 + 7_200),
            "2h ago"
        );
    }
}
