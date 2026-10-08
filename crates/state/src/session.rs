use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Error;
use gpui::{Context, Entity, EventEmitter, Task};
use music::{
    Capabilities, MusicApi, MusicProvider, PlaybackFactory, ProviderSession, Shape, SignIn,
    SignInFailure, SignInProblem, UserProfile,
};

use crate::Shelf;
use crate::catalog::CatalogSource;
use crate::settings::AppSettings;
use crate::{Io, Network, join};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Failure {
    pub problem: Option<SignInProblem>,
    pub summary: String,
    pub detail: Option<String>,
}

impl Failure {
    fn new(error: &Error) -> Self {
        let reason = format!("{error:#}");
        let problem = error
            .downcast_ref::<SignInFailure>()
            .map(|failure| failure.0)
            .or_else(|| music::trouble::offline(&reason).then_some(SignInProblem::Network));
        let detail = error
            .chain()
            .skip(1)
            .map(ToString::to_string)
            .collect::<Vec<_>>();
        Self {
            problem,
            summary: error.to_string(),
            detail: (!detail.is_empty()).then(|| detail.join(": ")),
        }
    }

    /// Whether the sign-in failed because there was no network, rather than because the account
    /// was refused. A provider that was only unreachable is still the user's.
    pub fn offline(&self) -> bool {
        self.problem == Some(SignInProblem::Network)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionState {
    SignedOut,
    Restoring,
    Authorizing,
    SignedIn(UserProfile),
    /// The stored account could not be reached. It is still the user's, so nothing is signed
    /// out: the app stays on whatever the library kept and on the local files, and tries the
    /// account again by itself once the network is back.
    Offline(Failure),
    Failed(Failure),
}

pub enum SessionEvent {
    SignedIn,
    SignedOut,
    LocalChanged,
}

pub struct ProviderInfo {
    pub slug: &'static str,
    pub name: &'static str,
    pub options: Vec<SignIn>,
    pub stored: bool,
    pub active: bool,
    pub pending: bool,
    pub error: Option<Failure>,
}

pub struct Session {
    state: SessionState,
    providers: Vec<Arc<dyn MusicProvider>>,
    active: Option<usize>,
    awaiting: Option<usize>,
    resume: Option<(usize, UserProfile)>,
    error: Option<(usize, Failure)>,
    settings: Entity<AppSettings>,
    client: Option<Arc<dyn MusicApi>>,
    catalog: Option<Arc<CatalogSource>>,
    playback: Option<Arc<dyn PlaybackFactory>>,
    shape: Shape,
    authenticated: bool,
    capabilities: Capabilities,
    io: Io,
    task: Option<Task<()>>,
    local_provider: Arc<dyn MusicProvider>,
    local_folders: Vec<PathBuf>,
    local_client: Option<Arc<dyn MusicApi>>,
    local_catalog: Option<Arc<CatalogSource>>,
    local_playback: Option<Arc<dyn PlaybackFactory>>,
    local_capabilities: Capabilities,
    local_task: Option<Task<()>>,
    /// Whether a local scan is under way, so the UI can show its progress.
    scanning: bool,
}

impl EventEmitter<SessionEvent> for Session {}

impl Session {
    pub fn new(
        providers: Vec<Arc<dyn MusicProvider>>,
        local_provider: Arc<dyn MusicProvider>,
        settings: Entity<AppSettings>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        let remembered = settings.read(cx).provider().to_string();
        let local_folders = settings.read(cx).local_folders().to_vec();
        let active = providers
            .iter()
            .position(|provider| provider.slug() == remembered);
        let local_playback = local_provider.playback_factory();
        let mut session = Self {
            state: SessionState::SignedOut,
            providers,
            active,
            awaiting: None,
            resume: None,
            error: None,
            settings,
            client: None,
            catalog: None,
            playback: None,
            shape: Shape::Saved,
            authenticated: false,
            capabilities: Capabilities::NONE,
            io,
            task: None,
            local_provider,
            local_folders,
            local_client: None,
            local_catalog: None,
            local_playback,
            local_capabilities: Capabilities::NONE,
            local_task: None,
            scanning: false,
        };
        session.restore_local(cx);
        session
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn client(&self) -> Option<Arc<dyn MusicApi>> {
        self.client.clone()
    }

    pub fn playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        self.playback.clone()
    }

    pub fn local_client(&self) -> Option<Arc<dyn MusicApi>> {
        self.local_client.clone()
    }

    /// The client serving a shelf, if that shelf has a provider right now.
    pub fn client_of(&self, shelf: Shelf) -> Option<Arc<dyn MusicApi>> {
        match shelf {
            Shelf::Streaming => self.client.clone(),
            Shelf::Local => self.local_client.clone(),
        }
    }

    /// What a shelf's library is made of. The local shelf is always a catalog.
    pub fn shape_of(&self, shelf: Shelf) -> Shape {
        match shelf {
            Shelf::Streaming => self.shape,
            Shelf::Local => Shape::Catalog,
        }
    }

    pub(crate) fn catalog(&self, id: &str) -> Option<Arc<CatalogSource>> {
        match music::is_local_id(id) {
            true => self.local_catalog.clone(),
            false => self.catalog.clone(),
        }
    }

    pub fn local_playback(&self) -> Option<Arc<dyn PlaybackFactory>> {
        self.local_playback.clone()
    }

    /// The local provider itself, so a file can be read as a track without waiting for
    /// a scan: an explorer open may point outside every configured folder.
    pub fn local_provider(&self) -> Arc<dyn MusicProvider> {
        self.local_provider.clone()
    }

    pub fn local_paths(&self) -> Vec<String> {
        self.local_folders
            .iter()
            .map(|path| path.display().to_string())
            .collect()
    }

    pub fn providers(&self) -> impl Iterator<Item = ProviderInfo> + '_ {
        self.providers
            .iter()
            .enumerate()
            .map(|(index, provider)| ProviderInfo {
                slug: provider.slug(),
                name: provider.name(),
                options: provider.sign_in_options(),
                stored: provider.stored(),
                active: self.active == Some(index),
                pending: self.awaiting == Some(index),
                error: match &self.error {
                    Some((failed, failure)) if *failed == index => Some(failure.clone()),
                    _ => None,
                },
            })
    }

    pub fn connected(&self) -> impl Iterator<Item = ProviderInfo> + '_ {
        self.providers().filter(|info| info.stored)
    }

    pub fn forget(&mut self, slug: &str, cx: &mut Context<Self>) {
        let Some(index) = self
            .providers
            .iter()
            .position(|provider| provider.slug() == slug)
        else {
            return;
        };
        if self.active == Some(index) {
            return self.sign_out(cx);
        }
        self.providers[index].sign_out();
        cx.notify();
    }

    pub fn switch(&mut self, slug: &str, cx: &mut Context<Self>) {
        if self.is_pending() {
            return;
        }
        let Some(index) = self
            .providers
            .iter()
            .position(|provider| provider.slug() == slug)
        else {
            return;
        };
        if self.active == Some(index) && matches!(self.state, SessionState::SignedIn(_)) {
            return;
        }
        self.release(cx);
        self.active = Some(index);
        self.restore(cx);
    }

    pub fn provider_name(&self) -> Option<&'static str> {
        let provider = &self.providers[self.active?];
        Some(provider.name())
    }

    pub fn provider_slug(&self) -> Option<&'static str> {
        let provider = &self.providers[self.active?];
        Some(provider.slug())
    }

    /// The host to try when checking whether the network is back. It is the active provider's
    /// own, so the check never touches a service the app is not already using.
    pub fn reach(&self) -> Option<String> {
        self.providers.get(self.active?)?.reach()
    }

    pub fn local_slug(&self) -> &'static str {
        self.local_provider.slug()
    }

    pub fn active_slugs(&self) -> Vec<&'static str> {
        let mut slugs = Vec::new();
        if self.client.is_some() {
            slugs.extend(self.provider_slug());
        }
        if self.local_client.is_some() {
            slugs.push(self.local_slug());
        }
        slugs
    }

    pub fn slug_for(&self, id: &str) -> Option<&'static str> {
        self.provider_for(id).map(|provider| provider.slug())
    }

    /// The provider an id belongs to, whichever shelf it sits on.
    pub(crate) fn provider_for(&self, id: &str) -> Option<&dyn MusicProvider> {
        match music::is_local_id(id) {
            true => Some(self.local_provider.as_ref()),
            false => self.active.map(|index| self.providers[index].as_ref()),
        }
    }

    pub fn authenticated(&self) -> bool {
        self.authenticated
    }

    /// What the live streaming provider can do beyond listing and playing. Nothing is offered
    /// while signed out, which is what the empty set means.
    pub fn capabilities(&self) -> Capabilities {
        self.capabilities
    }

    /// The same, for whichever shelf a thing belongs to. Anything that routes by id asks this
    /// one, since the two shelves can differ: local files keep favorites but seed no station.
    pub fn capabilities_of(&self, shelf: Shelf) -> Capabilities {
        match shelf {
            Shelf::Streaming => self.capabilities,
            Shelf::Local => self.local_capabilities,
        }
    }

    pub fn is_pending(&self) -> bool {
        matches!(
            self.state,
            SessionState::Restoring | SessionState::Authorizing
        )
    }

    pub fn restore(&mut self, cx: &mut Context<Self>) {
        if self.is_pending() {
            return;
        }
        let Some(active) = self.active else {
            self.state = SessionState::SignedOut;
            cx.notify();
            cx.emit(SessionEvent::SignedOut);
            return;
        };
        self.state = SessionState::Restoring;
        cx.notify();

        let provider = self.providers[active].clone();
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let restored = join(io.spawn(async move { provider.restore().await })).await;

            this.update(cx, |this, cx| match restored {
                Ok(Some(session)) => this.signed_in(session, active, cx),
                Ok(None) => {
                    this.state = SessionState::SignedOut;
                    cx.notify();
                    cx.emit(SessionEvent::SignedOut);
                }
                Err(error) => this.failed(&error, cx),
            })
            .ok();
        }));
    }

    pub fn sign_in(&mut self, slug: &str, method: SignIn, cx: &mut Context<Self>) {
        if self.is_pending() {
            return;
        }
        let Some(index) = self
            .providers
            .iter()
            .position(|provider| provider.slug() == slug)
        else {
            return;
        };
        self.resume = match &self.state {
            SessionState::SignedIn(profile) => self.active.map(|active| (active, profile.clone())),
            _ => None,
        };
        self.error = None;
        self.awaiting = Some(index);
        self.state = SessionState::Authorizing;
        cx.notify();

        let provider = self.providers[index].clone();
        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let authorized = join(io.spawn(async move { provider.sign_in(method).await })).await;

            this.update(cx, |this, cx| match authorized {
                Ok(session) => this.signed_in(session, index, cx),
                Err(error) => this.failed(&error, cx),
            })
            .ok();
        }));
    }

    pub fn cancel_sign_in(&mut self, cx: &mut Context<Self>) {
        if !matches!(self.state, SessionState::Authorizing) {
            return;
        }
        self.task = None;
        self.awaiting = None;
        self.error = None;
        if let Some((index, profile)) = self.resume.take() {
            self.active = Some(index);
            self.state = SessionState::SignedIn(profile);
            cx.notify();
            return;
        }
        self.state = SessionState::SignedOut;
        cx.notify();
        cx.emit(SessionEvent::SignedOut);
    }

    pub fn sign_out(&mut self, cx: &mut Context<Self>) {
        if let Some(active) = self.active {
            self.providers[active].sign_out();
        }
        self.release(cx);
        if let Some(index) = self.remaining() {
            self.active = Some(index);
            self.restore(cx);
        }
    }

    /// Whether the active provider has an account stored, which is what tells a failed restore
    /// from a user who signed out.
    fn stored_active(&self) -> bool {
        self.active
            .is_some_and(|index| self.providers[index].stored())
    }

    /// Tries the stored account again once the network is back, for a run that started without
    /// one. Anything but a run held up by the network is left alone.
    pub fn restore_if_offline(&mut self, cx: &mut Context<Self>) {
        if matches!(self.state, SessionState::Offline(_)) {
            self.restore(cx);
        }
    }

    fn remaining(&self) -> Option<usize> {
        self.active
            .filter(|index| self.providers[*index].stored())
            .or_else(|| self.providers.iter().position(|provider| provider.stored()))
    }

    fn release(&mut self, cx: &mut Context<Self>) {
        self.task = None;
        self.awaiting = None;
        self.resume = None;
        self.client = None;
        self.catalog = None;
        self.playback = None;
        self.shape = Shape::Saved;
        self.authenticated = false;
        self.capabilities = Capabilities::NONE;
        self.state = SessionState::SignedOut;
        cx.notify();
        cx.emit(SessionEvent::SignedOut);
    }

    fn signed_in(&mut self, session: ProviderSession, index: usize, cx: &mut Context<Self>) {
        let replaced = self
            .resume
            .take()
            .is_some_and(|(held, profile)| held != index || profile.id != session.profile.id);
        if replaced {
            self.drop_previous_session(cx);
        }
        self.active = Some(index);
        self.awaiting = None;
        self.error = None;
        let slug = self.providers[index].slug();
        self.settings.update(cx, |settings, cx| {
            settings.set_provider(slug, cx);
        });
        self.catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.client = Some(session.api);
        self.playback = Some(session.playback);
        self.shape = session.shape;
        self.authenticated = session.authenticated;
        self.capabilities = session.capabilities;
        self.state = SessionState::SignedIn(session.profile);
        cx.notify();
        cx.emit(SessionEvent::SignedIn);
    }

    fn drop_previous_session(&mut self, cx: &mut Context<Self>) {
        self.client = None;
        self.catalog = None;
        self.playback = None;
        self.shape = Shape::Saved;
        self.authenticated = false;
        self.capabilities = Capabilities::NONE;
        cx.emit(SessionEvent::SignedOut);
    }

    fn failed(&mut self, error: &Error, cx: &mut Context<Self>) {
        let failure = Failure::new(error);
        Network::failed(&format!("{error:#}"), cx);
        if let Some(failed) = self.awaiting.or(self.active) {
            self.error = Some((failed, failure.clone()));
        }
        // Only a restore may end up offline. A sign-in the user is watching says what went
        // wrong on the page they started it from.
        let restoring = self.awaiting.is_none();
        self.awaiting = None;
        if let Some((index, profile)) = self.resume.take() {
            self.active = Some(index);
            self.state = SessionState::SignedIn(profile);
            cx.notify();
            return;
        }
        if restoring && failure.offline() && self.stored_active() {
            log::warn!("session: the account could not be reached, carrying on offline");
            self.state = SessionState::Offline(failure);
            cx.notify();
            return;
        }
        self.client = None;
        self.catalog = None;
        self.playback = None;
        self.state = SessionState::Failed(failure);
        cx.notify();
        cx.emit(SessionEvent::SignedOut);
    }

    fn restore_local(&mut self, cx: &mut Context<Self>) {
        if self.local_folders.is_empty() {
            return;
        }
        self.rescan_local(false, cx);
    }

    /// Adds a folder to the local library, then rescans every configured folder together so
    /// artists and albums that span more than one root merge into one, seamlessly.
    pub fn add_local_folder(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.add_local_folders(vec![path], cx);
    }

    /// Same as [`Session::add_local_folder`], for a batch picked in one native dialog.
    pub fn add_local_folders(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        let mut folders = self.local_folders.clone();
        for path in paths {
            if folders.iter().any(|existing| overlaps(existing, &path)) {
                log::warn!(
                    "session: {} overlaps an already-added local folder",
                    path.display()
                );
                continue;
            }
            folders.push(path);
        }
        if folders.len() == self.local_folders.len() {
            return;
        }
        self.set_local_folders(folders, cx);
    }

    pub fn remove_local_folder(&mut self, path: &Path, cx: &mut Context<Self>) {
        let mut folders = self.local_folders.clone();
        let before = folders.len();
        folders.retain(|existing| existing != path);
        if folders.len() == before {
            return;
        }
        self.set_local_folders(folders, cx);
    }

    /// Rescans every configured local folder without changing the list, e.g. after files
    /// changed on disk or a tag was edited. A `thorough` rescan is the one the user asked for:
    /// it forgets what the last scan recorded, so every folder is listed and every file stat'd
    /// again, which is the only way an edit made behind Sonora's back is noticed.
    pub fn rescan_local(&mut self, thorough: bool, cx: &mut Context<Self>) {
        if thorough {
            self.local_provider.forget_scan();
        }
        self.set_local_folders(self.local_folders.clone(), cx);
    }

    /// Points the local library at `folders` and scans them. A scan already under way is
    /// cancelled first: its folders may be the ones just removed, and two scans would only
    /// fight over the same disk.
    fn set_local_folders(&mut self, folders: Vec<PathBuf>, cx: &mut Context<Self>) {
        music::progress::cancel();
        if folders.is_empty() {
            self.scanning = false;
            self.local_provider.sign_out();
            self.local_folders = Vec::new();
            self.settings.update(cx, |settings, cx| {
                settings.set_local_folders(Vec::new(), cx)
            });
            self.local_client = None;
            self.local_catalog = None;
            self.local_capabilities = Capabilities::NONE;
            self.local_task = None;
            cx.notify();
            cx.emit(SessionEvent::LocalChanged);
            return;
        }

        let provider = self.local_provider.clone();
        let chosen = folders.clone();
        let io = self.io.clone();
        self.scanning = true;
        cx.notify();
        self.local_task = Some(cx.spawn(async move |this, cx| {
            let signed_in =
                join(io.spawn(async move { provider.sign_in(SignIn::Path(folders)).await })).await;

            this.update(cx, |this, cx| {
                this.scanning = false;
                match signed_in {
                    Ok(session) => {
                        this.local_folders = chosen.clone();
                        this.settings
                            .update(cx, |settings, cx| settings.set_local_folders(chosen, cx));
                        this.local_signed_in(session, cx);
                    }
                    Err(error) => {
                        log::warn!("session: cannot update local music folders: {error:#}");
                        cx.notify();
                    }
                }
            })
            .ok();
        }));
    }

    /// Whether a local scan is under way right now.
    pub fn scanning(&self) -> bool {
        self.scanning
    }

    fn local_signed_in(&mut self, session: ProviderSession, cx: &mut Context<Self>) {
        self.local_catalog = Some(Arc::new(CatalogSource::new(session.api.clone())));
        self.local_client = Some(session.api);
        if self.local_playback.is_none() {
            self.local_playback = Some(session.playback);
        }
        self.local_capabilities = session.capabilities;
        cx.notify();
        cx.emit(SessionEvent::LocalChanged);
    }
}

/// Whether `a` and `b` are the same directory, or one contains the other — either way, scanning
/// both would double-count the tracks they share.
fn overlaps(a: &Path, b: &Path) -> bool {
    let a = std::fs::canonicalize(a).unwrap_or_else(|_| a.to_path_buf());
    let b = std::fs::canonicalize(b).unwrap_or_else(|_| b.to_path_buf());
    a == b || a.starts_with(&b) || b.starts_with(&a)
}
