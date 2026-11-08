use serde_json::{Value, json};
use std::collections::BTreeMap;
#[derive(Clone, Default)]
pub(super) struct Event {
    pub kind: String,
    pub line: usize,
    pub text: String,
    pub file: String,
    pub reload: Option<String>,
    pub session: Option<String>,
    pub role: Option<String>,
    pub detail: Option<String>,
}
impl Event {
    pub fn json(&self) -> Value {
        json!({"kind":self.kind,"line":self.line,"text":self.text.trim(),"file":self.file,"reload_id":self.reload,"session_id":self.session,"role":self.role,"detail":self.detail})
    }
}
#[derive(Default)]
pub(super) struct Record {
    pub reload: String,
    pub session: String,
    pub role: Option<String>,
    pub events: BTreeMap<String, Vec<Event>>,
    pub status: Option<String>,
    pub delivered_at: Value,
    pub markers: Value,
}
impl Record {
    pub fn events(&self, key: &str) -> &[Event] {
        self.events.get(key).map(Vec::as_slice).unwrap_or(&[])
    }
    pub fn client_queued(&self) -> bool {
        !self.events("history_queued").is_empty()
            || self.events("tui_flow").iter().any(|e| {
                e.detail.as_ref().is_some_and(|s| {
                    let s = s.to_lowercase();
                    s.contains("queued") || s.contains("outcome=resumed")
                })
            })
    }
    pub fn accepted(&self) -> bool {
        !self.events("delivered").is_empty()
            || self.events("tui_flow").iter().any(|e| {
                e.detail.as_ref().is_some_and(|s| {
                    s.contains("accepted_by=client_message_accepted")
                        || s.contains("accepted_by=server_startup_headless")
                })
            })
    }
    pub fn verdict(&self) -> &'static str {
        if self.client_queued() {
            "ok-client-queued"
        } else if self.accepted() {
            "ok-server-accepted"
        } else if matches!(self.status.as_deref(), Some("pending" | "Pending")) {
            "pending"
        } else if !self.events("claimed").is_empty()
            || self
                .status
                .as_ref()
                .is_some_and(|s| s.eq_ignore_ascii_case("delivered"))
        {
            "suspect-delivered-without-acceptance-log"
        } else if !self.events("attached").is_empty() {
            "attached-not-accepted"
        } else if !self.events("persisted").is_empty() {
            "persisted-not-attached"
        } else {
            "observed-without-persist-log"
        }
    }
    pub fn json(&self, lines: bool) -> Value {
        let mut out = json!({"reload_id":self.reload,"session_id":self.session,"role":self.role,"durable_status":self.status,"durable_delivered_at":self.delivered_at,
            "persisted":!self.events("persisted").is_empty(),"claimed":!self.events("claimed").is_empty(),"attached_count":self.events("attached").len(),"delivered_count":self.events("delivered").len(),"delivery_mismatch_count":self.events("delivery_mismatch").len(),"history_snapshot_count":self.events("history_snapshot").len(),"history_queued_count":self.events("history_queued").len(),"tui_flow_count":self.events("tui_flow").len(),"send_history_count":self.events("send_history").len(),"member_running_after_reload_count":self.events("member_running").len(),"transcript_markers":if self.markers.is_null(){json!({})}else{self.markers.clone()},"verdict":self.verdict()});
        if lines {
            out["events"] = json!(
                [
                    "persisted",
                    "claimed",
                    "attached",
                    "delivered",
                    "delivery_mismatch",
                    "history_snapshot",
                    "history_queued",
                    "tui_flow",
                    "send_history",
                    "member_running"
                ]
                .into_iter()
                .flat_map(|k| self.events(k))
                .map(Event::json)
                .collect::<Vec<_>>()
            );
        }
        out
    }
}
pub(super) type Records = BTreeMap<(String, String), Record>;
pub(super) fn add(records: &mut Records, event: Event) {
    let (Some(reload), Some(session)) = (&event.reload, &event.session) else {
        return;
    };
    if reload.is_empty() || session.is_empty() {
        return;
    }
    let r = records
        .entry((reload.clone(), session.clone()))
        .or_insert_with(|| Record {
            reload: reload.clone(),
            session: session.clone(),
            ..Default::default()
        });
    if r.role.is_none() {
        r.role = event.role.clone();
    }
    let events = r.events.entry(event.kind.clone()).or_default();
    if matches!(event.kind.as_str(), "persisted" | "claimed") {
        events.clear();
    }
    events.push(event);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn verdict_precedence_requires_acceptance_evidence() {
        let mut r = Record::default();
        assert_eq!(r.verdict(), "observed-without-persist-log");
        r.events.insert("persisted".into(), vec![Event::default()]);
        assert_eq!(r.verdict(), "persisted-not-attached");
        r.events.insert("attached".into(), vec![Event::default()]);
        assert_eq!(r.verdict(), "attached-not-accepted");
        r.status = Some("Delivered".into());
        assert!(r.verdict().starts_with("suspect"));
        r.status = Some("Pending".into());
        assert_eq!(r.verdict(), "pending");
        r.events.insert("delivered".into(), vec![Event::default()]);
        assert_eq!(r.verdict(), "ok-server-accepted");
        r.events
            .insert("history_queued".into(), vec![Event::default()]);
        assert_eq!(r.verdict(), "ok-client-queued");
    }
}
