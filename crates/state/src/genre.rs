use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;

use gpui::{App, Context, Entity, Task};
use music::{Album, Genre, GenreDetail, GenreItem, GenreSection, Shape, Track};
use tokio::task::AbortHandle;

use crate::{Io, Library, Session, SessionEvent, Shelf, join};

/// One genre of the songs the library holds, counted from their tags.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShelfGenre {
    pub name: String,
    pub songs: usize,
    pub albums: usize,
    pub cover: Option<String>,
}

/// The genres of the streaming shelf's own songs, kept in step with the library, and the songs
/// and albums of the genre page open. Only a `Catalog` shelf holds every song, so a `Saved` one,
/// whose library is the favorites, lists none.
pub struct ShelfGenres {
    library: Entity<Library>,
    genres: Vec<ShelfGenre>,
    open: Option<String>,
    songs: Vec<Track>,
    albums: Vec<Album>,
}

impl ShelfGenres {
    pub fn new(library: Entity<Library>, cx: &mut Context<Self>) -> Self {
        cx.observe(&library, |this, _, cx| this.collect(cx))
            .detach();
        let mut genres = Self {
            library,
            genres: Vec::new(),
            open: None,
            songs: Vec::new(),
            albums: Vec::new(),
        };
        genres.collect(cx);
        genres
    }

    /// Whether the shelf holds the whole catalogue, so a genres page means something.
    pub fn available(&self, cx: &App) -> bool {
        self.library.read(cx).shape(Shelf::Streaming) == Shape::Catalog
    }

    pub fn genres(&self) -> &[ShelfGenre] {
        &self.genres
    }

    pub fn is_loading(&self, cx: &App) -> bool {
        self.library
            .read(cx)
            .loading(Shelf::Streaming, crate::LibraryPart::Tracks)
    }

    /// The genre whose page is open, as the library spells it.
    pub fn name(&self) -> Option<&str> {
        self.open.as_deref()
    }

    /// The open genre's songs, by artist, album, disc and track.
    pub fn songs(&self) -> &[Track] {
        &self.songs
    }

    /// The open genre's albums, newest first.
    pub fn albums(&self) -> &[Album] {
        &self.albums
    }

    pub fn open(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.open.as_deref() == Some(name) {
            return;
        }
        self.open = Some(name.to_owned());
        self.songs.clear();
        self.albums.clear();
        self.collect(cx);
        cx.notify();
    }

    /// Recounts the genres and refills the open page from what the library holds now. Notifies
    /// only when something changed, since the library notifies for far more than its songs.
    fn collect(&mut self, cx: &mut Context<Self>) {
        let library = self.library.read(cx);
        let state = library.state(Shelf::Streaming);
        let (genres, songs, albums) = match library.shape(Shelf::Streaming) {
            Shape::Catalog => {
                let tracks = state.tracks();
                let genres = count(tracks);
                let (songs, albums) = match self.open.as_deref() {
                    Some(name) => gather(name, tracks, state.albums()),
                    None => (Vec::new(), Vec::new()),
                };
                (genres, songs, albums)
            }
            Shape::Saved => (Vec::new(), Vec::new(), Vec::new()),
        };
        if genres == self.genres && songs == self.songs && albums == self.albums {
            return;
        }
        self.genres = genres;
        self.songs = songs;
        self.albums = albums;
        cx.notify();
    }
}

pub struct Genres {
    genres: Rc<Vec<Genre>>,
    loading: bool,
    error: Option<String>,
    session: Entity<Session>,
    io: Io,
    task: Option<Task<()>>,
}

impl Genres {
    pub fn new(session: Entity<Session>, io: Io, cx: &mut Context<Self>) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn => this.load(cx),
            SessionEvent::SignedOut => {
                this.task = None;
                this.genres = Rc::new(Vec::new());
                this.loading = false;
                this.error = None;
                cx.notify();
            }
            SessionEvent::LocalChanged => {}
        })
        .detach();

        Self {
            genres: Rc::new(Vec::new()),
            loading: false,
            error: None,
            session,
            io,
            task: None,
        }
    }

    pub fn genres(&self) -> Rc<Vec<Genre>> {
        self.genres.clone()
    }

    pub fn adopt(&mut self, id: &str, cover: Option<String>, cx: &mut Context<Self>) {
        let Some(cover) = cover else {
            return;
        };
        let Some(genre) = Rc::make_mut(&mut self.genres)
            .iter_mut()
            .find(|genre| genre.id == id && genre.cover.is_none())
        else {
            return;
        };

        genre.cover = Some(cover);
        cx.notify();
    }

    pub fn forget(&mut self, id: &str, cx: &mut Context<Self>) {
        let kept = self.genres.len();
        Rc::make_mut(&mut self.genres).retain(|genre| genre.id != id);
        if self.genres.len() != kept {
            cx.notify();
        }
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn load(&mut self, cx: &mut Context<Self>) {
        if self.loading || !self.genres.is_empty() {
            return;
        }
        let Some(client) = self.session.read(cx).client() else {
            return;
        };

        self.loading = true;
        self.error = None;
        cx.notify();

        let io = self.io.clone();
        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(io.spawn(async move { client.genres().await })).await;

            this.update(cx, |this, cx| {
                this.loading = false;
                match crate::settled(loaded, cx) {
                    Ok(genres) => this.genres = Rc::new(genres),
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        }));
    }
}

pub struct GenreDetails {
    id: Option<String>,
    detail: Option<Arc<GenreDetail>>,
    sections: Rc<Vec<GenreSection>>,
    loading: bool,
    error: Option<String>,
    session: Entity<Session>,
    genres: Entity<Genres>,
    io: Io,
    task: Option<Task<()>>,
    request: Option<AbortHandle>,
}

impl GenreDetails {
    pub fn new(
        session: Entity<Session>,
        genres: Entity<Genres>,
        io: Io,
        cx: &mut Context<Self>,
    ) -> Self {
        cx.subscribe(&session, |this, _, event, cx| match event {
            SessionEvent::SignedIn => {
                if let Some(id) = this.id.clone() {
                    this.clear();
                    this.open(&id, cx);
                }
            }
            SessionEvent::SignedOut => {
                this.clear();
                cx.notify();
            }
            SessionEvent::LocalChanged => {}
        })
        .detach();

        Self {
            id: None,
            detail: None,
            sections: Rc::new(Vec::new()),
            loading: false,
            error: None,
            session,
            genres,
            io,
            task: None,
            request: None,
        }
    }

    pub fn id(&self) -> Option<&str> {
        self.id.as_deref()
    }

    pub fn name(&self) -> Option<&str> {
        self.detail.as_ref().map(|detail| detail.name.as_str())
    }

    pub fn sections(&self) -> Rc<Vec<GenreSection>> {
        self.sections.clone()
    }

    pub fn is_loading(&self) -> bool {
        self.loading
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn open(&mut self, id: &str, cx: &mut Context<Self>) {
        if self.id.as_deref() == Some(id) && (self.loading || self.detail.is_some()) {
            return;
        }

        self.clear();
        self.id = Some(id.to_owned());
        let Some(catalog) = self.session.read(cx).catalog(id) else {
            cx.notify();
            return;
        };
        if let Some(detail) = catalog.peek_genre(id) {
            self.adopt(id, detail, cx);
            cx.notify();
            return;
        }

        self.loading = true;
        cx.notify();

        let id = id.to_owned();
        let request = self.io.spawn({
            let id = id.clone();
            async move { catalog.genre(&id).await }
        });
        self.request = Some(request.abort_handle());
        self.task = Some(cx.spawn(async move |this, cx| {
            let loaded = join(request).await;

            this.update(cx, |this, cx| {
                if this.id.as_deref() != Some(id.as_str()) {
                    return;
                }

                this.loading = false;
                this.request = None;
                match crate::settled(loaded, cx) {
                    Ok(detail) => this.adopt(&id, detail, cx),
                    Err(reason) => this.error = Some(reason),
                }
                cx.notify();
            })
            .ok();
        }));
    }

    fn adopt(&mut self, id: &str, detail: Arc<GenreDetail>, cx: &mut Context<Self>) {
        let cover = pictured(&detail);
        self.genres
            .update(cx, |genres, cx| match detail.sections.is_empty() {
                true => genres.forget(id, cx),
                false => genres.adopt(id, cover, cx),
            });
        self.sections = Rc::new(detail.sections.clone());
        self.detail = Some(detail);
    }

    fn clear(&mut self) {
        self.task = None;
        if let Some(request) = self.request.take() {
            request.abort();
        }
        self.id = None;
        self.detail = None;
        self.sections = Rc::new(Vec::new());
        self.loading = false;
        self.error = None;
    }
}

/// Whether `track` is tagged with the genre `key`, a lowercased name.
fn tagged(track: &Track, key: &str) -> bool {
    track.tags.iter().any(|tag| tag.to_lowercase() == key)
}

/// Every genre the tracks carry, spelled as first seen and counted case-blind, the most
/// listened-to in songs first. The cover is the first song's of the genre that has one.
fn count(tracks: &[Track]) -> Vec<ShelfGenre> {
    struct Tally<'a> {
        name: &'a str,
        songs: usize,
        albums: HashSet<&'a str>,
        cover: Option<&'a str>,
    }

    let mut tallies: HashMap<String, Tally> = HashMap::new();
    for track in tracks {
        for tag in &track.tags {
            let tally = tallies.entry(tag.to_lowercase()).or_insert_with(|| Tally {
                name: tag,
                songs: 0,
                albums: HashSet::new(),
                cover: None,
            });
            tally.songs += 1;
            tally.albums.extend(track.album_id.as_deref());
            if tally.cover.is_none() {
                tally.cover = track.cover.as_deref();
            }
        }
    }

    let mut genres: Vec<ShelfGenre> = tallies
        .into_values()
        .map(|tally| ShelfGenre {
            name: tally.name.to_owned(),
            songs: tally.songs,
            albums: tally.albums.len(),
            cover: tally.cover.map(str::to_owned),
        })
        .collect();
    genres.sort_by(|a, b| b.songs.cmp(&a.songs).then_with(|| a.name.cmp(&b.name)));
    genres
}

/// The songs tagged `name` and the albums they sit on.
fn gather(name: &str, tracks: &[Track], albums: &[Album]) -> (Vec<Track>, Vec<Album>) {
    let key = name.to_lowercase();
    let mut songs: Vec<Track> = tracks
        .iter()
        .filter(|track| tagged(track, &key))
        .cloned()
        .collect();
    songs.sort_by_cached_key(|track| {
        (
            track.artists.to_lowercase(),
            track.album.to_lowercase(),
            track.disc_number,
            track.track_number,
        )
    });

    let held: HashSet<&str> = songs
        .iter()
        .filter_map(|track| track.album_id.as_deref())
        .collect();
    let mut albums: Vec<Album> = albums
        .iter()
        .filter(|album| held.contains(album.id.as_str()))
        .cloned()
        .collect();
    albums.sort_by(|a, b| b.year.cmp(&a.year).then_with(|| a.name.cmp(&b.name)));
    (songs, albums)
}

fn pictured(detail: &GenreDetail) -> Option<String> {
    let items = || detail.sections.iter().flat_map(|section| &section.items);
    let album = items().find_map(|item| match item {
        GenreItem::Album(album) => album.cover.clone(),
        _ => None,
    });

    album.or_else(|| {
        items().find_map(|item| match item {
            GenreItem::Playlist(playlist) => playlist.cover.clone(),
            _ => None,
        })
    })
}
