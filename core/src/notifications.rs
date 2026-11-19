//! # Notification service
//!
//! Multi-channel delivery: **email**, **push**, **in-app** and **SMS**.
//!
//! * **Templates** are versioned and localised. `{{name}}` substitutes an
//!   HTML-escaped value (on the email channel), `{{{name}}}` substitutes raw
//!   text. A missing variable is an error, never an empty string. Locale lookup
//!   falls back from `fr-CA` to `fr` to `en`; a request may pin a version,
//!   otherwise the latest is used. (MJML is compiled to HTML upstream and
//!   registered as the email body.)
//! * **Preferences** are per user: channels can be switched off, categories can
//!   be muted per channel, and each user has a locale. Transactional messages
//!   (password resets, security alerts) ignore opt-outs.
//! * **Unsubscribe** links carry an HMAC-signed token, so they cannot be forged
//!   or edited and need no login.
//! * **Scheduling, batching, tracking.** A request may carry `send_at`; batches
//!   report per-item failures instead of aborting; every message has a
//!   [`DeliveryRecord`] whose status follows `Scheduled/Queued -> Sent ->
//!   Delivered | Bounced | Failed` (or `Suppressed`), updated from provider
//!   callbacks through [`NotificationService::record_event`].
//! * **Queue.** `send` only renders and enqueues; [`NotificationService::process`]
//!   drains the [`DeliveryQueue`] (bind it to the job system in production) and
//!   calls the per-channel [`ChannelProvider`] (SendGrid/SES, FCM/APNs, Twilio,
//!   WebSocket/SSE adapters). Retryable provider errors are requeued up to
//!   `max_attempts`.
//! * **Test mode** renders and records messages in an outbox instead of
//!   contacting any provider, for development.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use thiserror::Error;

type HmacSha256 = Hmac<Sha256>;

pub const DEFAULT_LOCALE: &str = "en";
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;
/// Category value meaning "everything on this channel" in an unsubscribe.
pub const ALL_CATEGORIES: &str = "*";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Channel {
    Email,
    Push,
    InApp,
    Sms,
}

impl Channel {
    fn as_str(self) -> &'static str {
        match self {
            Channel::Email => "email",
            Channel::Push => "push",
            Channel::InApp => "in_app",
            Channel::Sms => "sms",
        }
    }

    fn parse(s: &str) -> Option<Channel> {
        match s {
            "email" => Some(Channel::Email),
            "push" => Some(Channel::Push),
            "in_app" => Some(Channel::InApp),
            "sms" => Some(Channel::Sms),
            _ => None,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum NotificationError {
    #[error("no template `{id}` for channel {channel:?}")]
    TemplateNotFound { id: String, channel: Channel },
    #[error("template `{id}` v{version} ({channel:?}, {locale}) is already registered")]
    DuplicateTemplate {
        id: String,
        version: u32,
        channel: Channel,
        locale: String,
    },
    #[error("template variable `{0}` was not provided")]
    MissingVariable(String),
    #[error("unterminated `{{{{` in template")]
    MalformedTemplate,
    #[error("no provider registered for {0:?}")]
    NoProvider(Channel),
    #[error("invalid or tampered unsubscribe token")]
    InvalidToken,
    #[error("unknown delivery `{0}`")]
    UnknownDelivery(String),
    #[error("notification state unavailable")]
    StatePoisoned,
}

// ---------------------------------------------------------------------------
// Templates
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Template {
    pub id: String,
    pub version: u32,
    pub channel: Channel,
    pub locale: String,
    pub subject: Option<String>,
    pub body: String,
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Substitute `{{var}}` (escaped when `escape` is set) and `{{{var}}}` (raw).
pub fn render(
    template: &str,
    vars: &BTreeMap<String, String>,
    escape: bool,
) -> Result<String, NotificationError> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find("{{") {
        out.push_str(&rest[..start]);
        let after = &rest[start..];
        let (raw, open, close) = if after.starts_with("{{{") {
            (true, 3, "}}}")
        } else {
            (false, 2, "}}")
        };
        let end = after[open..].find(close).ok_or(NotificationError::MalformedTemplate)?;
        let name = after[open..open + end].trim();
        let value = vars
            .get(name)
            .ok_or_else(|| NotificationError::MissingVariable(name.to_string()))?;
        if raw || !escape {
            out.push_str(value);
        } else {
            out.push_str(&escape_html(value));
        }
        rest = &after[open + end + close.len()..];
    }
    out.push_str(rest);
    Ok(out)
}

#[derive(Default)]
pub struct TemplateRegistry {
    templates: Vec<Template>,
}

impl TemplateRegistry {
    pub fn register(&mut self, template: Template) -> Result<(), NotificationError> {
        let duplicate = self.templates.iter().any(|t| {
            t.id == template.id
                && t.version == template.version
                && t.channel == template.channel
                && t.locale == template.locale
        });
        if duplicate {
            return Err(NotificationError::DuplicateTemplate {
                id: template.id,
                version: template.version,
                channel: template.channel,
                locale: template.locale,
            });
        }
        self.templates.push(template);
        Ok(())
    }

    /// Best template for `locale`, falling back `fr-CA -> fr -> en`. `version`
    /// pins a version; otherwise the latest wins.
    pub fn resolve(
        &self,
        id: &str,
        channel: Channel,
        locale: &str,
        version: Option<u32>,
    ) -> Result<&Template, NotificationError> {
        let language = locale.split(['-', '_']).next().unwrap_or(locale);
        let candidates = [locale, language, DEFAULT_LOCALE];

        for candidate in candidates {
            let best = self
                .templates
                .iter()
                .filter(|t| {
                    t.id == id
                        && t.channel == channel
                        && t.locale.eq_ignore_ascii_case(candidate)
                        && version.is_none_or(|v| t.version == v)
                })
                .max_by_key(|t| t.version);
            if let Some(template) = best {
                return Ok(template);
            }
        }
        Err(NotificationError::TemplateNotFound {
            id: id.to_string(),
            channel,
        })
    }
}

// ---------------------------------------------------------------------------
// Requests, records, providers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct NotificationRequest {
    pub user_id: String,
    /// Email address, phone number or device token; in-app uses the user id.
    pub recipient: String,
    pub channel: Channel,
    pub category: String,
    /// Transactional messages ignore opt-outs and unsubscribes.
    pub transactional: bool,
    pub template_id: String,
    pub template_version: Option<u32>,
    pub vars: BTreeMap<String, String>,
    pub locale: Option<String>,
    /// Epoch seconds. In the past (or `None`) means send now.
    pub send_at: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RenderedMessage {
    pub delivery_id: String,
    pub channel: Channel,
    pub to: String,
    pub subject: Option<String>,
    pub body: String,
    pub locale: String,
    pub template_id: String,
    pub template_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    Scheduled,
    Queued,
    Sent,
    Delivered,
    Bounced,
    Failed,
    Suppressed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DeliveryRecord {
    pub id: String,
    pub user_id: String,
    pub channel: Channel,
    pub category: String,
    pub status: DeliveryStatus,
    pub attempts: u32,
    pub scheduled_for: Option<u64>,
    pub provider_message_id: Option<String>,
    pub detail: Option<String>,
}

/// Provider callbacks that move a record along.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DeliveryEvent {
    Delivered,
    Bounced(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderError {
    pub message: String,
    pub retryable: bool,
}

/// Adapter for one channel's delivery backend.
pub trait ChannelProvider: Send + Sync {
    fn channel(&self) -> Channel;
    /// Returns the provider's message id.
    fn send(&self, message: &RenderedMessage) -> Result<String, ProviderError>;
}

/// Where accepted deliveries wait for a worker.
pub trait DeliveryQueue: Send + Sync {
    fn push(&self, delivery_id: String);
    fn pop(&self) -> Option<String>;
}

#[derive(Default)]
pub struct InMemoryQueue {
    items: Mutex<VecDeque<String>>,
}

impl DeliveryQueue for InMemoryQueue {
    fn push(&self, delivery_id: String) {
        if let Ok(mut items) = self.items.lock() {
            items.push_back(delivery_id);
        }
    }

    fn pop(&self) -> Option<String> {
        self.items.lock().ok().and_then(|mut items| items.pop_front())
    }
}

// ---------------------------------------------------------------------------
// Preferences
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
struct Preferences {
    disabled_channels: HashSet<Channel>,
    /// `(channel, category)`; category [`ALL_CATEGORIES`] covers the channel.
    unsubscribed: HashSet<(Channel, String)>,
    locale: Option<String>,
}

impl Preferences {
    fn suppression(&self, channel: Channel, category: &str) -> Option<&'static str> {
        if self.disabled_channels.contains(&channel) {
            Some("channel disabled by user")
        } else if self.unsubscribed.contains(&(channel, ALL_CATEGORIES.to_string()))
            || self.unsubscribed.contains(&(channel, category.to_string()))
        {
            Some("unsubscribed")
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Service
// ---------------------------------------------------------------------------

#[derive(Default)]
struct State {
    templates: TemplateRegistry,
    preferences: HashMap<String, Preferences>,
    records: HashMap<String, DeliveryRecord>,
    messages: HashMap<String, RenderedMessage>,
    scheduled: Vec<(u64, String)>,
    test_outbox: Vec<RenderedMessage>,
}

#[derive(Debug, Default)]
pub struct BatchReport {
    pub accepted: Vec<String>,
    pub failed: Vec<(usize, NotificationError)>,
}

pub struct NotificationService {
    state: Mutex<State>,
    providers: HashMap<Channel, Arc<dyn ChannelProvider>>,
    queue: Arc<dyn DeliveryQueue>,
    secret: Vec<u8>,
    test_mode: bool,
    max_attempts: u32,
    next_id: AtomicU64,
}

impl NotificationService {
    pub fn new(secret: &[u8], queue: Arc<dyn DeliveryQueue>) -> Self {
        NotificationService {
            state: Mutex::new(State::default()),
            providers: HashMap::new(),
            queue,
            secret: secret.to_vec(),
            test_mode: false,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            next_id: AtomicU64::new(1),
        }
    }

    pub fn with_provider(mut self, provider: Arc<dyn ChannelProvider>) -> Self {
        self.providers.insert(provider.channel(), provider);
        self
    }

    /// Render and record messages without contacting any provider.
    pub fn with_test_mode(mut self, enabled: bool) -> Self {
        self.test_mode = enabled;
        self
    }

    pub fn with_max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts.max(1);
        self
    }

    fn state(&self) -> Result<MutexGuard<'_, State>, NotificationError> {
        self.state.lock().map_err(|_| NotificationError::StatePoisoned)
    }

    pub fn register_template(&self, template: Template) -> Result<(), NotificationError> {
        self.state()?.templates.register(template)
    }

    // -- preferences ------------------------------------------------------

    pub fn set_channel_enabled(&self, user_id: &str, channel: Channel, enabled: bool) -> Result<(), NotificationError> {
        let mut state = self.state()?;
        let prefs = state.preferences.entry(user_id.to_string()).or_default();
        if enabled {
            prefs.disabled_channels.remove(&channel);
        } else {
            prefs.disabled_channels.insert(channel);
        }
        Ok(())
    }

    pub fn set_locale(&self, user_id: &str, locale: &str) -> Result<(), NotificationError> {
        self.state()?
            .preferences
            .entry(user_id.to_string())
            .or_default()
            .locale = Some(locale.to_string());
        Ok(())
    }

    // -- unsubscribe ------------------------------------------------------

    fn mac(&self, data: &str) -> Result<HmacSha256, NotificationError> {
        let mut mac = HmacSha256::new_from_slice(&self.secret).map_err(|_| NotificationError::InvalidToken)?;
        mac.update(data.as_bytes());
        Ok(mac)
    }

    /// Signed token for an unsubscribe link. `category` may be [`ALL_CATEGORIES`].
    pub fn unsubscribe_token(&self, user_id: &str, channel: Channel, category: &str) -> Result<String, NotificationError> {
        let body = URL_SAFE_NO_PAD.encode(format!("{user_id}\n{}\n{category}", channel.as_str()));
        let tag = self.mac(&body)?.finalize().into_bytes();
        Ok(format!("{body}.{}", URL_SAFE_NO_PAD.encode(tag)))
    }

    /// Verify a token and record the opt-out. Returns what was unsubscribed.
    pub fn unsubscribe(&self, token: &str) -> Result<(String, Channel, String), NotificationError> {
        let (body, tag) = token.split_once('.').ok_or(NotificationError::InvalidToken)?;
        let tag = URL_SAFE_NO_PAD.decode(tag).map_err(|_| NotificationError::InvalidToken)?;
        self.mac(body)?.verify_slice(&tag).map_err(|_| NotificationError::InvalidToken)?;

        let decoded = URL_SAFE_NO_PAD.decode(body).map_err(|_| NotificationError::InvalidToken)?;
        let decoded = String::from_utf8(decoded).map_err(|_| NotificationError::InvalidToken)?;
        let mut parts = decoded.splitn(3, '\n');
        let (user, channel, category) = match (parts.next(), parts.next(), parts.next()) {
            (Some(u), Some(c), Some(cat)) => (u.to_string(), Channel::parse(c).ok_or(NotificationError::InvalidToken)?, cat.to_string()),
            _ => return Err(NotificationError::InvalidToken),
        };

        self.state()?
            .preferences
            .entry(user.clone())
            .or_default()
            .unsubscribed
            .insert((channel, category.clone()));
        Ok((user, channel, category))
    }

    // -- sending ----------------------------------------------------------

    fn new_id(&self) -> String {
        format!("ntf_{}", self.next_id.fetch_add(1, Ordering::SeqCst))
    }

    /// Render, check preferences and enqueue (or schedule) one notification.
    /// Returns the delivery id to track it by.
    pub fn send(&self, request: &NotificationRequest, now: u64) -> Result<String, NotificationError> {
        let id = self.new_id();
        let mut state = self.state()?;

        let locale = request
            .locale
            .clone()
            .or_else(|| state.preferences.get(&request.user_id).and_then(|p| p.locale.clone()))
            .unwrap_or_else(|| DEFAULT_LOCALE.to_string());

        // Render first so a bad template or missing variable fails the call
        // instead of producing a half-sent message.
        let template = state
            .templates
            .resolve(&request.template_id, request.channel, &locale, request.template_version)?
            .clone();
        let escape = request.channel == Channel::Email;
        let message = RenderedMessage {
            delivery_id: id.clone(),
            channel: request.channel,
            to: request.recipient.clone(),
            subject: template
                .subject
                .as_deref()
                .map(|s| render(s, &request.vars, false))
                .transpose()?,
            body: render(&template.body, &request.vars, escape)?,
            locale,
            template_id: template.id.clone(),
            template_version: template.version,
        };

        let suppression = if request.transactional {
            None
        } else {
            state
                .preferences
                .get(&request.user_id)
                .and_then(|p| p.suppression(request.channel, &request.category))
        };

        let mut record = DeliveryRecord {
            id: id.clone(),
            user_id: request.user_id.clone(),
            channel: request.channel,
            category: request.category.clone(),
            status: DeliveryStatus::Queued,
            attempts: 0,
            scheduled_for: None,
            provider_message_id: None,
            detail: None,
        };

        if let Some(reason) = suppression {
            record.status = DeliveryStatus::Suppressed;
            record.detail = Some(reason.to_string());
            state.records.insert(id.clone(), record);
            return Ok(id);
        }

        state.messages.insert(id.clone(), message);
        match request.send_at.filter(|at| *at > now) {
            Some(at) => {
                record.status = DeliveryStatus::Scheduled;
                record.scheduled_for = Some(at);
                state.scheduled.push((at, id.clone()));
            }
            None => self.queue.push(id.clone()),
        }
        state.records.insert(id.clone(), record);
        Ok(id)
    }

    /// Send many; one bad request does not stop the rest.
    pub fn send_batch(&self, requests: &[NotificationRequest], now: u64) -> BatchReport {
        let mut report = BatchReport::default();
        for (index, request) in requests.iter().enumerate() {
            match self.send(request, now) {
                Ok(id) => report.accepted.push(id),
                Err(error) => report.failed.push((index, error)),
            }
        }
        report
    }

    /// Worker step: release due scheduled messages, then deliver up to `limit`
    /// queued ones. Returns how many were handled.
    pub fn process(&self, now: u64, limit: usize) -> Result<usize, NotificationError> {
        {
            let mut state = self.state()?;
            let (due, later): (Vec<_>, Vec<_>) = state.scheduled.drain(..).partition(|(at, _)| *at <= now);
            state.scheduled = later;
            for (_, id) in due {
                if let Some(record) = state.records.get_mut(&id) {
                    record.status = DeliveryStatus::Queued;
                }
                self.queue.push(id);
            }
        }

        let mut handled = 0;
        while handled < limit {
            let Some(id) = self.queue.pop() else { break };
            self.deliver(&id)?;
            handled += 1;
        }
        Ok(handled)
    }

    fn deliver(&self, id: &str) -> Result<(), NotificationError> {
        let message = {
            let state = self.state()?;
            state.messages.get(id).cloned()
        }
        .ok_or_else(|| NotificationError::UnknownDelivery(id.to_string()))?;

        if self.test_mode {
            let mut state = self.state()?;
            state.test_outbox.push(message);
            if let Some(record) = state.records.get_mut(id) {
                record.status = DeliveryStatus::Sent;
                record.attempts += 1;
                record.detail = Some("test mode: not sent".to_string());
            }
            return Ok(());
        }

        let provider = self.providers.get(&message.channel);
        let result = match provider {
            Some(provider) => provider.send(&message),
            None => Err(ProviderError {
                message: NotificationError::NoProvider(message.channel).to_string(),
                retryable: false,
            }),
        };

        let mut state = self.state()?;
        let mut requeue = false;
        if let Some(record) = state.records.get_mut(id) {
            record.attempts += 1;
            match result {
                Ok(provider_id) => {
                    record.status = DeliveryStatus::Sent;
                    record.provider_message_id = Some(provider_id);
                    record.detail = None;
                }
                Err(error) if error.retryable && record.attempts < self.max_attempts => {
                    record.status = DeliveryStatus::Queued;
                    record.detail = Some(error.message);
                    requeue = true;
                }
                Err(error) => {
                    record.status = DeliveryStatus::Failed;
                    record.detail = Some(error.message);
                }
            }
        }
        drop(state);
        if requeue {
            self.queue.push(id.to_string());
        }
        Ok(())
    }

    // -- tracking ---------------------------------------------------------

    /// Apply a provider callback (delivery receipt, bounce).
    pub fn record_event(&self, delivery_id: &str, event: DeliveryEvent) -> Result<(), NotificationError> {
        let mut state = self.state()?;
        let record = state
            .records
            .get_mut(delivery_id)
            .ok_or_else(|| NotificationError::UnknownDelivery(delivery_id.to_string()))?;
        match event {
            DeliveryEvent::Delivered => record.status = DeliveryStatus::Delivered,
            DeliveryEvent::Bounced(reason) => {
                record.status = DeliveryStatus::Bounced;
                record.detail = Some(reason);
            }
        }
        Ok(())
    }

    pub fn delivery(&self, delivery_id: &str) -> Result<Option<DeliveryRecord>, NotificationError> {
        Ok(self.state()?.records.get(delivery_id).cloned())
    }

    /// Everything rendered while in test mode.
    pub fn test_outbox(&self) -> Result<Vec<RenderedMessage>, NotificationError> {
        Ok(self.state()?.test_outbox.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeProvider {
        channel: Channel,
        sent: Mutex<Vec<RenderedMessage>>,
        failures: Mutex<Vec<ProviderError>>,
    }

    impl FakeProvider {
        fn new(channel: Channel) -> Arc<Self> {
            Arc::new(FakeProvider {
                channel,
                sent: Mutex::new(Vec::new()),
                failures: Mutex::new(Vec::new()),
            })
        }
        fn fail_next(&self, error: ProviderError) {
            self.failures.lock().unwrap().push(error);
        }
    }

    impl ChannelProvider for FakeProvider {
        fn channel(&self) -> Channel {
            self.channel
        }
        fn send(&self, message: &RenderedMessage) -> Result<String, ProviderError> {
            if let Some(error) = self.failures.lock().unwrap().pop() {
                return Err(error);
            }
            self.sent.lock().unwrap().push(message.clone());
            Ok(format!("prov-{}", message.delivery_id))
        }
    }

    fn template(id: &str, version: u32, channel: Channel, locale: &str, body: &str) -> Template {
        Template {
            id: id.into(),
            version,
            channel,
            locale: locale.into(),
            subject: (channel == Channel::Email).then(|| "Hi {{name}}".to_string()),
            body: body.into(),
        }
    }

    fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect()
    }

    fn request(channel: Channel) -> NotificationRequest {
        NotificationRequest {
            user_id: "u1".into(),
            recipient: "u1@example.com".into(),
            channel,
            category: "marketing".into(),
            transactional: false,
            template_id: "welcome".into(),
            template_version: None,
            vars: vars(&[("name", "Ada")]),
            locale: None,
            send_at: None,
        }
    }

    fn service_with(provider: &Arc<FakeProvider>) -> NotificationService {
        let svc = NotificationService::new(b"secret", Arc::new(InMemoryQueue::default()))
            .with_provider(provider.clone());
        svc.register_template(template("welcome", 1, Channel::Email, "en", "Hello {{name}}")).unwrap();
        svc.register_template(template("welcome", 1, Channel::Sms, "en", "Hi {{name}}")).unwrap();
        svc
    }

    #[test]
    fn rendering_escapes_html_for_double_braces_only() {
        let v = vars(&[("x", "<b>&\"'")]);
        assert_eq!(render("a {{x}} b", &v, true).unwrap(), "a &lt;b&gt;&amp;&quot;&#39; b");
        assert_eq!(render("a {{{x}}} b", &v, true).unwrap(), "a <b>&\"' b");
        assert_eq!(render("a {{x}} b", &v, false).unwrap(), "a <b>&\"' b");
        assert_eq!(render("no vars", &v, true).unwrap(), "no vars");
    }

    #[test]
    fn rendering_fails_on_missing_variables_and_broken_templates() {
        assert_eq!(render("{{ nope }}", &vars(&[]), true), Err(NotificationError::MissingVariable("nope".into())));
        assert_eq!(render("{{oops", &vars(&[]), true), Err(NotificationError::MalformedTemplate));
    }

    #[test]
    fn locale_falls_back_by_language_then_english_and_versions_resolve_to_latest() {
        let mut reg = TemplateRegistry::default();
        reg.register(template("t", 1, Channel::Sms, "en", "en v1")).unwrap();
        reg.register(template("t", 2, Channel::Sms, "en", "en v2")).unwrap();
        reg.register(template("t", 1, Channel::Sms, "fr", "fr v1")).unwrap();

        assert_eq!(reg.resolve("t", Channel::Sms, "fr-CA", None).unwrap().body, "fr v1");
        assert_eq!(reg.resolve("t", Channel::Sms, "de", None).unwrap().body, "en v2");
        assert_eq!(reg.resolve("t", Channel::Sms, "en", Some(1)).unwrap().body, "en v1");
        assert!(reg.resolve("t", Channel::Email, "en", None).is_err());
        assert!(reg.register(template("t", 1, Channel::Sms, "en", "dup")).is_err());
    }

    #[test]
    fn send_queues_then_process_delivers_and_tracks_status() {
        let provider = FakeProvider::new(Channel::Email);
        let svc = service_with(&provider);

        let id = svc.send(&request(Channel::Email), 100).unwrap();
        assert_eq!(svc.delivery(&id).unwrap().unwrap().status, DeliveryStatus::Queued);

        assert_eq!(svc.process(100, 10).unwrap(), 1);
        let record = svc.delivery(&id).unwrap().unwrap();
        assert_eq!(record.status, DeliveryStatus::Sent);
        assert_eq!(record.provider_message_id.as_deref(), Some(format!("prov-{id}").as_str()));

        let sent = provider.sent.lock().unwrap();
        assert_eq!(sent[0].body, "Hello Ada");
        assert_eq!(sent[0].subject.as_deref(), Some("Hi Ada"));
        drop(sent);

        svc.record_event(&id, DeliveryEvent::Delivered).unwrap();
        assert_eq!(svc.delivery(&id).unwrap().unwrap().status, DeliveryStatus::Delivered);
        svc.record_event(&id, DeliveryEvent::Bounced("mailbox full".into())).unwrap();
        assert_eq!(svc.delivery(&id).unwrap().unwrap().detail.as_deref(), Some("mailbox full"));
        assert!(svc.record_event("nope", DeliveryEvent::Delivered).is_err());
    }

    #[test]
    fn email_bodies_are_html_escaped_but_sms_bodies_are_not() {
        let email = FakeProvider::new(Channel::Email);
        let svc = service_with(&email);
        let mut req = request(Channel::Email);
        req.vars = vars(&[("name", "<script>")]);
        svc.send(&req, 0).unwrap();
        svc.process(0, 10).unwrap();
        assert_eq!(email.sent.lock().unwrap()[0].body, "Hello &lt;script&gt;");

        let sms = FakeProvider::new(Channel::Sms);
        let svc = service_with(&sms);
        let mut req = request(Channel::Sms);
        req.vars = vars(&[("name", "<b>")]);
        svc.send(&req, 0).unwrap();
        svc.process(0, 10).unwrap();
        assert_eq!(sms.sent.lock().unwrap()[0].body, "Hi <b>");
    }

    #[test]
    fn scheduled_messages_wait_until_due() {
        let provider = FakeProvider::new(Channel::Email);
        let svc = service_with(&provider);
        let mut req = request(Channel::Email);
        req.send_at = Some(500);

        let id = svc.send(&req, 100).unwrap();
        assert_eq!(svc.delivery(&id).unwrap().unwrap().status, DeliveryStatus::Scheduled);

        assert_eq!(svc.process(499, 10).unwrap(), 0);
        assert!(provider.sent.lock().unwrap().is_empty());

        assert_eq!(svc.process(500, 10).unwrap(), 1);
        assert_eq!(svc.delivery(&id).unwrap().unwrap().status, DeliveryStatus::Sent);
    }

    #[test]
    fn preferences_and_unsubscribes_suppress_but_transactional_mail_still_goes() {
        let provider = FakeProvider::new(Channel::Email);
        let svc = service_with(&provider);

        svc.set_channel_enabled("u1", Channel::Email, false).unwrap();
        let id = svc.send(&request(Channel::Email), 0).unwrap();
        let record = svc.delivery(&id).unwrap().unwrap();
        assert_eq!(record.status, DeliveryStatus::Suppressed);
        assert_eq!(record.detail.as_deref(), Some("channel disabled by user"));

        let mut transactional = request(Channel::Email);
        transactional.transactional = true;
        let id = svc.send(&transactional, 0).unwrap();
        assert_eq!(svc.delivery(&id).unwrap().unwrap().status, DeliveryStatus::Queued);

        svc.set_channel_enabled("u1", Channel::Email, true).unwrap();
        let token = svc.unsubscribe_token("u1", Channel::Email, "marketing").unwrap();
        assert_eq!(svc.unsubscribe(&token).unwrap(), ("u1".to_string(), Channel::Email, "marketing".to_string()));
        let id = svc.send(&request(Channel::Email), 0).unwrap();
        assert_eq!(svc.delivery(&id).unwrap().unwrap().status, DeliveryStatus::Suppressed);

        let mut other = request(Channel::Email);
        other.category = "product".into();
        let id = svc.send(&other, 0).unwrap();
        assert_eq!(svc.delivery(&id).unwrap().unwrap().status, DeliveryStatus::Queued);
    }

    #[test]
    fn unsubscribe_tokens_cannot_be_forged_or_edited() {
        let svc = service_with(&FakeProvider::new(Channel::Email));
        let token = svc.unsubscribe_token("u1", Channel::Email, ALL_CATEGORIES).unwrap();

        let forged_body = URL_SAFE_NO_PAD.encode("victim\nemail\n*");
        let tag = token.split_once('.').unwrap().1;
        for bad in [format!("{forged_body}.{tag}"), "junk".to_string(), String::new(), format!("{token}x")] {
            assert_eq!(svc.unsubscribe(&bad), Err(NotificationError::InvalidToken), "{bad}");
        }
        let other = NotificationService::new(b"other-secret", Arc::new(InMemoryQueue::default()));
        assert_eq!(other.unsubscribe(&token), Err(NotificationError::InvalidToken));
    }

    #[test]
    fn user_locale_preference_selects_the_template() {
        let provider = FakeProvider::new(Channel::Sms);
        let svc = service_with(&provider);
        svc.register_template(template("welcome", 1, Channel::Sms, "fr", "Salut {{name}}")).unwrap();
        svc.set_locale("u1", "fr-CA").unwrap();

        svc.send(&request(Channel::Sms), 0).unwrap();
        svc.process(0, 10).unwrap();

        assert_eq!(provider.sent.lock().unwrap()[0].body, "Salut Ada");
    }

    #[test]
    fn batch_sending_reports_failures_per_item() {
        let svc = service_with(&FakeProvider::new(Channel::Email));
        let mut bad = request(Channel::Email);
        bad.template_id = "missing".into();
        let mut also_bad = request(Channel::Email);
        also_bad.vars = BTreeMap::new();

        let report = svc.send_batch(&[request(Channel::Email), bad, also_bad, request(Channel::Email)], 0);

        assert_eq!(report.accepted.len(), 2);
        assert_eq!(report.failed.len(), 2);
        assert_eq!(report.failed[0].0, 1);
        assert!(matches!(report.failed[0].1, NotificationError::TemplateNotFound { .. }));
        assert_eq!(report.failed[1], (2, NotificationError::MissingVariable("name".into())));
    }

    #[test]
    fn retryable_provider_errors_are_retried_up_to_the_limit() {
        let provider = FakeProvider::new(Channel::Email);
        let svc = service_with(&provider).with_max_attempts(3);
        let flaky = ProviderError { message: "timeout".into(), retryable: true };
        provider.fail_next(flaky.clone());
        provider.fail_next(flaky);

        let id = svc.send(&request(Channel::Email), 0).unwrap();
        svc.process(0, 10).unwrap();

        let record = svc.delivery(&id).unwrap().unwrap();
        assert_eq!(record.status, DeliveryStatus::Sent);
        assert_eq!(record.attempts, 3);
    }

    #[test]
    fn permanent_errors_and_exhausted_retries_fail_the_delivery() {
        let provider = FakeProvider::new(Channel::Email);
        let svc = service_with(&provider).with_max_attempts(2);

        provider.fail_next(ProviderError { message: "invalid address".into(), retryable: false });
        let id = svc.send(&request(Channel::Email), 0).unwrap();
        svc.process(0, 10).unwrap();
        let record = svc.delivery(&id).unwrap().unwrap();
        assert_eq!((record.status, record.attempts), (DeliveryStatus::Failed, 1));
        assert_eq!(record.detail.as_deref(), Some("invalid address"));

        let flaky = ProviderError { message: "timeout".into(), retryable: true };
        provider.fail_next(flaky.clone());
        provider.fail_next(flaky);
        let id = svc.send(&request(Channel::Email), 0).unwrap();
        svc.process(0, 10).unwrap();
        let record = svc.delivery(&id).unwrap().unwrap();
        assert_eq!((record.status, record.attempts), (DeliveryStatus::Failed, 2));
    }

    #[test]
    fn a_channel_without_a_provider_fails_cleanly() {
        let svc = service_with(&FakeProvider::new(Channel::Email));
        let id = svc.send(&request(Channel::Sms), 0).unwrap();
        svc.process(0, 10).unwrap();
        let record = svc.delivery(&id).unwrap().unwrap();
        assert_eq!(record.status, DeliveryStatus::Failed);
        assert!(record.detail.unwrap().contains("no provider"));
    }

    #[test]
    fn test_mode_captures_messages_without_calling_providers() {
        let provider = FakeProvider::new(Channel::Email);
        let svc = service_with(&provider).with_test_mode(true);

        let id = svc.send(&request(Channel::Email), 0).unwrap();
        svc.process(0, 10).unwrap();

        assert!(provider.sent.lock().unwrap().is_empty());
        assert_eq!(svc.test_outbox().unwrap()[0].body, "Hello Ada");
        assert_eq!(svc.delivery(&id).unwrap().unwrap().detail.as_deref(), Some("test mode: not sent"));
    }
}
