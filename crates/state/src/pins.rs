use std::cell::OnceCell;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::rc::Rc;

use gpui::{App, Context, Entity};
use ui::{Pin, PinKind};

use crate::library::{Library, LibraryEvent, Shelf};
use crate::session::Session;
use crate::settings::AppSettings;

/// An automatic order for the pinned list. No sort at all means the order the user dragged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PinSort {
    Alphabetical,
    Kind,
}

impl PinSort {
    pub const ALL: [Self; 2] = [Self::Alphabetical, Self::Kind];

    /// The stored id. Anything else, the empty string included, means the dragged order.
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|sort| sort.id() == id)
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Alphabetical => "alphabetical",
            Self::Kind => "kind",
        }
    }

    pub fn key(self) -> &'static str {
        match self {
            Self::Alphabetical => "nav-pins-alphabetical",
            Self::Kind => "nav-pins-kind",
        }
    }
}

/// The one pinned list, whatever provider each entry came from. Order is kept locally and covers
/// every entry at once, so a provider that cannot reorder its own pins still follows the drag, and
/// a streamed pin can sit between two local ones.
///
pub struct Pins {
    settings: Entity<AppSettings>,
    library: Entity<Library>,
    session: Entity<Session>,
    /// `entries` as last laid out. Cleared whenever the settings, the session or the library
    /// move, so a frame reads the list without rebuilding it.
    laid: OnceCell<Rc<Vec<Pin>>>,
    /// `library` as last laid out, cleared alongside `laid`.
    rest: OnceCell<Rc<Vec<Pin>>>,
    /// The digest of what the shelves last listed, so a library notify that changed nothing
    /// the sidebar shows does not rebuild it.
    seen: u64,
}

impl Pins {
    pub fn new(
        settings: Entity<AppSettings>,
        library: Entity<Library>,
        session: Entity<Session>,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&library, |this, _, event, cx| {
            let LibraryEvent::PlaylistGone(id) = event else {
                return;
            };
            let pin = Pin::new(PinKind::Playlist, id, "");
            if let Some(slug) = this.session.read(cx).slug_for(id) {
                this.settings
                    .update(cx, |settings, cx| settings.unpin(slug, &pin, cx));
            }
            this.changed(cx);
        })
        .detach();
        cx.observe(&library, |this, _, cx| {
            let seen = this.fingerprint(cx);
            if this.seen != seen {
                this.seen = seen;
                this.changed(cx);
            }
        })
        .detach();
        cx.observe(&settings, |this, _, cx| this.changed(cx))
            .detach();
        cx.observe(&session, |this, _, cx| this.changed(cx))
            .detach();

        Self {
            settings,
            library,
            session,
            laid: OnceCell::new(),
            rest: OnceCell::new(),
            seen: 0,
        }
    }

    /// Every pin of the live providers, laid out the way the user asked for. The list is built
    /// once per change and shared afterwards.
    pub fn entries(&self, cx: &App) -> Rc<Vec<Pin>> {
        self.laid
            .get_or_init(|| {
                let slugs = self.session.read(cx).active_slugs();
                let mut pinned = self.settings.read(cx).pinned(&slugs);
                self.lay_out(&mut pinned, cx);
                Rc::new(pinned)
            })
            .clone()
    }

    /// Everything the live shelves hold that is not pinned: albums, artists and playlists, laid
    /// out the same way the pins above them are. Nothing else a provider lists belongs here,
    /// since the sidebar can only open these three. Built once per change, like `entries`.
    pub fn library(&self, cx: &App) -> Rc<Vec<Pin>> {
        self.rest
            .get_or_init(|| Rc::new(self.gather_library(cx)))
            .clone()
    }

    /// A digest of the albums, artists and playlists the live shelves list, with the names
    /// and covers the sidebar shows for them.
    fn fingerprint(&self, cx: &App) -> u64 {
        let mut hasher = DefaultHasher::new();
        let library = self.library.read(cx);
        for shelf in [Shelf::Streaming, Shelf::Local] {
            if self.session.read(cx).client_of(shelf).is_none() {
                continue;
            }
            let held = library.state(shelf);
            for playlist in held.playlists() {
                (&playlist.id, &playlist.name, &playlist.cover).hash(&mut hasher);
            }
            for album in held.albums() {
                (&album.id, &album.name, &album.cover_large, &album.cover).hash(&mut hasher);
            }
            for artist in held.artists() {
                (&artist.id, &artist.name, &artist.cover).hash(&mut hasher);
            }
        }
        hasher.finish()
    }

    /// Drops both laid-out lists so the next read rebuilds them.
    fn forget(&mut self) {
        self.laid.take();
        self.rest.take();
    }

    /// Forgets the laid-out lists and tells the observers.
    fn changed(&mut self, cx: &mut Context<Self>) {
        self.forget();
        cx.notify();
    }

    fn gather_library(&self, cx: &App) -> Vec<Pin> {
        let pinned = self.dragged(cx);
        let library = self.library.read(cx);
        let mut rest = Vec::new();
        for shelf in [Shelf::Streaming, Shelf::Local] {
            if self.session.read(cx).client_of(shelf).is_none() {
                continue;
            }
            let held = library.state(shelf);
            rest.extend(held.playlists().iter().map(|playlist| {
                Pin::new(PinKind::Playlist, &playlist.id, &playlist.name)
                    .cover(playlist.cover.clone())
            }));
            rest.extend(held.albums().iter().map(|album| {
                Pin::new(PinKind::Album, &album.id, &album.name)
                    .cover(album.cover_large.clone().or_else(|| album.cover.clone()))
            }));
            rest.extend(held.artists().iter().map(|artist| {
                Pin::new(PinKind::Artist, &artist.id, &artist.name).cover(artist.cover.clone())
            }));
        }
        rest.retain(|pin| !pinned.iter().any(|held| held.same(pin)));
        self.lay_out(&mut rest, cx);
        rest
    }

    /// The pins in the order they are stored in, whatever the current sort shows.
    fn dragged(&self, cx: &App) -> Vec<Pin> {
        let slugs = self.session.read(cx).active_slugs();
        self.settings.read(cx).pinned(&slugs)
    }

    /// The automatic order in effect, or `None` for the order the user dragged.
    pub fn sort(&self, cx: &App) -> Option<PinSort> {
        self.settings.read(cx).sidebar_pin_sort()
    }

    pub fn reversed(&self, cx: &App) -> bool {
        self.settings.read(cx).sidebar_pin_reversed()
    }

    pub fn sorted(&self, cx: &App) -> bool {
        self.sort(cx).is_some()
    }

    /// Applies the automatic order, if there is one. Without one the list keeps the order it
    /// arrived in, which for the pins is the one the user dragged.
    fn lay_out(&self, pins: &mut [Pin], cx: &App) {
        match self.sort(cx) {
            Some(PinSort::Alphabetical) => pins.sort_by_key(|pin| pin.title.to_lowercase()),
            Some(PinSort::Kind) => {
                pins.sort_by_key(|pin| (rank(pin.kind), pin.title.to_lowercase()))
            }
            None => return,
        }
        if self.reversed(cx) {
            pins.reverse();
        }
    }

    /// Steps one order along: off, then down, then off again. Picking another order starts it
    /// from the top.
    pub fn choose(&mut self, sort: PinSort, cx: &mut Context<Self>) {
        let next = match (self.sort(cx) == Some(sort), self.reversed(cx)) {
            (true, false) => Some((sort, true)),
            (true, true) => None,
            (false, _) => Some((sort, false)),
        };
        let (sort, reversed) = match next {
            Some((sort, reversed)) => (Some(sort), reversed),
            None => (None, false),
        };
        self.settings.update(cx, |settings, cx| {
            settings.set_sidebar_pin_sort(sort, reversed, cx)
        });
        self.changed(cx);
    }

    pub fn holds(&self, pin: &Pin, cx: &App) -> bool {
        self.dragged(cx).iter().any(|held| held.same(pin))
    }

    pub fn toggle(&mut self, pin: Pin, cx: &mut Context<Self>) {
        match self.holds(&pin, cx) {
            true => self.unpin(pin, cx),
            false => self.place(pin, None, cx),
        }
    }

    /// Pins at `gap`, a slot counted among the pins already shown, or moves an existing pin there.
    /// `None` appends, and never moves a pin that is already in the list.
    pub fn place(&mut self, pin: Pin, gap: Option<usize>, cx: &mut Context<Self>) {
        let Some(slug) = self.session.read(cx).slug_for(&pin.id) else {
            return;
        };
        let known = self.holds(&pin, cx);
        match gap {
            Some(_) => self.settle(cx),
            None if known => return,
            None => {}
        }
        let slugs = self.session.read(cx).active_slugs();
        self.settings.update(cx, |settings, cx| {
            settings.pin(slug, pin.clone(), gap, &slugs, cx)
        });
        self.changed(cx);
    }

    pub fn unpin(&mut self, pin: Pin, cx: &mut Context<Self>) {
        let Some(slug) = self.session.read(cx).slug_for(&pin.id) else {
            return;
        };
        self.settings
            .update(cx, |settings, cx| settings.unpin(slug, &pin, cx));
        self.changed(cx);
    }

    /// Freezes the list the way it reads on screen and goes back to the dragged order, so a slot
    /// the user aimed at means the same thing in the store as it did on screen.
    fn settle(&mut self, cx: &mut Context<Self>) {
        if !self.sorted(cx) {
            return;
        }
        let shown = self.entries(cx);
        let slugs = self.session.read(cx).active_slugs();
        self.settings.update(cx, |settings, cx| {
            settings.rearrange(&shown, &slugs, cx);
            settings.set_sidebar_pin_sort(None, false, cx);
        });
        self.forget();
    }
}

fn rank(kind: PinKind) -> u8 {
    match kind {
        PinKind::Playlist => 0,
        PinKind::Album => 1,
        PinKind::Artist => 2,
        PinKind::Song => 3,
    }
}
