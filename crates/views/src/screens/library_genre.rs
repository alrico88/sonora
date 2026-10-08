use gpui::prelude::*;
use gpui::{
    AnyElement, App, Context, Entity, FontWeight, Pixels, Render, ScrollHandle, SharedString,
    Window, div,
};
use i18n::t;
use music::Track;
use state::{AppSettings, Playback, ShelfGenres, Sonora};
use ui::{
    ActiveTheme as _, Button, MIN_CONTENT, Scrollbar, Scroller, TableDelegate, TableEvent,
    TableState, Text, Viewport, scrolled, table, vacant,
};

use crate::chrome::Chrome;
use crate::shared::album_grid::{AlbumGrid, CardGrid};
use crate::shared::cells;
use crate::shared::hero::{HeroMetaStrip, HeroPlayButton, PageHero};
use crate::shared::page;
use crate::shared::tracks::{
    LIBRARY_COLUMNS, PlaybackStatus, TrackSource, Tracks, playback_status,
};

/// The settings key the songs table's layout and sort are stored under.
const SECTION: &str = "library-genre";
/// How many rows of albums show before "Show all".
const ALBUM_ROWS: usize = 2;
/// The page's children before the songs table, which the table's viewport is measured from.
const TABLE_ITEM: usize = 3;

/// The open genre's songs, as the library holds them.
struct GenreSongs {
    genres: Entity<ShelfGenres>,
}

impl Tracks for GenreSongs {
    fn tracks<'a>(&self, cx: &'a App) -> &'a [Track] {
        self.genres.read(cx).songs()
    }

    fn is_loading(&self, cx: &App) -> bool {
        let genres = self.genres.read(cx);
        genres.songs().is_empty() && genres.is_loading(cx)
    }
}

/// One genre of the library's own songs: its albums, then every song tagged with it.
pub(crate) struct LibraryGenreView {
    genres: Entity<ShelfGenres>,
    playback: Entity<Playback>,
    playback_status: PlaybackStatus,
    settings: Entity<AppSettings>,
    scrollbar: Entity<Scrollbar>,
    songs: Entity<TableState<TrackSource>>,
    width: Pixels,
    albums_open: bool,
}

impl LibraryGenreView {
    pub(crate) fn new(playback: Entity<Playback>, cx: &mut Context<Self>) -> Self {
        let genres = Sonora::global(cx).shelf_genres.clone();
        let library = Sonora::global(cx).library.clone();
        let settings = Sonora::global(cx).settings.clone();
        let id = cx.entity_id();
        let scrollbar = cx.new(|_| Scrollbar::new(ScrollHandle::new()).watching(id));
        let scroll = scrollbar.read(cx).scroll().clone();
        let saved = settings.read(cx).table(SECTION);
        let sorting = settings.read(cx).sorting(SECTION);

        let songs = cx.new(|cx| {
            let menu_scrollbar = cx.new(|_| Scrollbar::inset().watching(id));
            let source = TrackSource::new(
                LIBRARY_COLUMNS,
                GenreSongs {
                    genres: genres.clone(),
                },
                playback.clone(),
                menu_scrollbar,
                cx,
            )
            .with_liked(library.clone());
            let source = source.table(cx.weak_entity());
            let mut delegate = TableDelegate::new(source, MIN_CONTENT, cx);
            delegate.set_layout(saved, cx);
            delegate.set_sorting(sorting.flatten(), cx);
            TableState::new(delegate, cx).follow(scroll)
        });

        cx.observe(&genres, |this, _, cx| {
            this.songs.update(cx, |table, cx| table.rebuild(cx));
            cx.notify();
        })
        .detach();
        cx.observe(&library, |this, _, cx| {
            this.songs.update(cx, |table, cx| table.refresh(cx));
            cx.notify();
        })
        .detach();
        cx.observe(&playback, |this, playback, cx| {
            let current = playback_status(&playback, cx);
            if this.playback_status == current {
                return;
            }
            this.playback_status = current;
            this.songs.update(cx, |table, cx| table.refresh(cx));
            cx.notify();
        })
        .detach();
        let chrome = Chrome::entity(cx);
        cx.observe(&chrome, |_, _, cx| cx.notify()).detach();

        cx.subscribe(&songs, |this, _, event, cx| match event {
            TableEvent::DoubleClicked(display) => {
                page::play(&this.songs, &this.playback, *display, cx)
            }
            TableEvent::Activated(display) => {
                page::play_or_toggle(&this.songs, &this.playback, *display, cx)
            }
            TableEvent::Removed => {}
            _ => page::store(
                &this.settings.clone(),
                &this.songs.clone(),
                SECTION,
                SECTION,
                cx,
            ),
        })
        .detach();

        let playback_status = playback_status(&playback, cx);
        Self {
            genres,
            playback,
            playback_status,
            settings,
            scrollbar,
            songs,
            width: MIN_CONTENT,
            albums_open: false,
        }
    }

    /// Opens the page of `name`, back at the top with the albums folded.
    pub(crate) fn open(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.genres.read(cx).name() != Some(name) {
            self.albums_open = false;
            self.scrollbar
                .read(cx)
                .scroll()
                .set_offset(gpui::Point::default());
        }
        self.genres.update(cx, |genres, cx| genres.open(name, cx));
    }

    fn hero(&self, name: SharedString, cx: &App) -> AnyElement {
        let genres = self.genres.read(cx);
        let strip = HeroMetaStrip::new()
            .text(t!("count-songs", count = genres.songs().len()))
            .text(t!("count-albums", count = genres.albums().len()));

        PageHero::new("library-genre-hero", name)
            .fallback("icons/guitar.svg")
            .accent()
            .eyebrow(t!("library-genre-eyebrow"))
            .meta(strip)
            .actions(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .child(HeroPlayButton::listed(
                        "play-library-genre",
                        t!("common-play"),
                        &self.songs,
                        self.playback.clone(),
                    ))
                    .child(HeroPlayButton::shuffle_listed(
                        "shuffle-library-genre",
                        &self.songs,
                        self.playback.clone(),
                    )),
            )
            .into_any_element()
    }

    /// The genre's albums, folded to a couple of rows until "Show all" opens them.
    fn albums(&self, room: Pixels, cx: &mut Context<Self>) -> Option<AnyElement> {
        let theme = *cx.theme();
        let albums = self.genres.read(cx).albums();
        if albums.is_empty() {
            return None;
        }
        let folded = ALBUM_ROWS * CardGrid::layout(room).columns;
        let shown = match self.albums_open {
            true => albums.len(),
            false => folded.min(albums.len()),
        };
        let grid = AlbumGrid::new(
            "library-genre-album",
            room,
            albums.iter().take(shown).cloned().enumerate(),
            self.playback.clone(),
        );
        let more = (albums.len() > folded).then(|| {
            Button::new("library-genre-albums-more")
                .ghost()
                .small()
                .label(match self.albums_open {
                    true => t!("library-genre-albums-less"),
                    false => t!("library-genre-albums-more"),
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.albums_open = !this.albums_open;
                    cx.notify();
                }))
        });

        Some(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .pt_6()
                .child(title(t!("library-genre-albums"), &theme))
                .child(grid)
                .children(more)
                .into_any_element(),
        )
    }
}

impl Render for LibraryGenreView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = *cx.theme();
        let inset = theme.metrics.inset;
        page::resize(&self.songs, &mut self.width, inset, window, cx);
        let room = cells::content_width(window, page::reserved(inset), cx);

        let Some(name) = self.genres.read(cx).name().map(str::to_owned) else {
            return div().size_full();
        };
        let empty = self.genres.read(cx).songs().is_empty() && !self.genres.read(cx).is_loading(cx);

        let scroll = self.scrollbar.read(cx).scroll().clone();
        if let Some(bounds) = scroll.bounds_for_item(TABLE_ITEM) {
            // item bounds are laid out unscrolled, so this is how far the page has scrolled
            // past the table's top edge
            let view = scroll.bounds();
            let top = scrolled(&scroll) - (bounds.origin.y - view.origin.y);
            self.songs.update(cx, |table, _| {
                table.set_viewport(Viewport::measured(top, view.size.height, window))
            });
        }

        let hero = self.hero(SharedString::from(name), cx);
        let albums = self
            .albums(room, cx)
            .unwrap_or_else(|| div().into_any_element());

        div().size_full().child(
            Scroller::new("library-genre", &self.scrollbar)
                .px(inset)
                .pt(inset)
                .pb(inset)
                .child(hero)
                .child(albums)
                .child(
                    div()
                        .pt_6()
                        .pb_3()
                        .child(title(t!("library-genre-songs"), &theme)),
                )
                .child(
                    table(&self.songs)
                        .rounded(theme.radius)
                        .border_1()
                        .border_color(theme.border),
                )
                .when(empty, |this| this.child(vacant(t!("genre-empty"), cx))),
        )
    }
}

fn title(label: SharedString, theme: &ui::Theme) -> impl IntoElement {
    div()
        .text_size(theme.text(Text::Title))
        .font_weight(FontWeight::BOLD)
        .child(label)
}
