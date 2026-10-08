use std::cmp::Ordering;

use gpui::{AnyElement, App, Entity, SharedString, TextAlign};
use state::{Shelf, ShelfGenre, ShelfGenres};
use ui::ActiveTheme as _;
use ui::rank::{ESSENTIAL, SPARE};
use ui::{Cell, ColumnSpec, TableSource, Width};

use crate::shared::cells::{self, NUMBER, TRAILING};
use crate::shared::text::{folded, holds};
use crate::shared::tracks::initial;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum GenreField {
    Index,
    Cover,
    Name,
    Albums,
    Songs,
}

const COLUMN: ColumnSpec<GenreField> = ColumnSpec::filling(GenreField::Index);

const INDEX: ColumnSpec<GenreField> = ColumnSpec::numbering(GenreField::Index, NUMBER);

const COVER: ColumnSpec<GenreField> = ColumnSpec::artwork(GenreField::Cover);

const NAME: ColumnSpec<GenreField> = ColumnSpec {
    field: GenreField::Name,
    key: "name",
    header: "column-name",
    rank: ESSENTIAL,
    ..COLUMN
};

const ALBUMS: ColumnSpec<GenreField> = ColumnSpec {
    field: GenreField::Albums,
    key: "albums",
    header: "column-albums",
    align: TextAlign::Right,
    width: Width::Fixed(TRAILING),
    rank: SPARE,
    ..COLUMN
};

const SONGS: ColumnSpec<GenreField> = ColumnSpec {
    field: GenreField::Songs,
    key: "songs",
    header: "column-songs",
    align: TextAlign::Right,
    width: Width::Fixed(TRAILING),
    rank: SPARE,
    ..COLUMN
};

pub(super) const COLUMNS: &[ColumnSpec<GenreField>] = &[INDEX, COVER, NAME, ALBUMS, SONGS];

/// The genres of a shelf's songs. Only the streaming shelf counts them, so a local shelf lists
/// none.
pub(super) struct GenreSource {
    genres: Entity<ShelfGenres>,
    shelf: Shelf,
}

impl GenreSource {
    pub(super) fn shelved(genres: Entity<ShelfGenres>, shelf: Shelf) -> Self {
        Self { genres, shelf }
    }

    pub(super) fn at(&self, row: usize, cx: &App) -> Option<ShelfGenre> {
        self.genres(cx).get(row).cloned()
    }

    fn genres<'a>(&self, cx: &'a App) -> &'a [ShelfGenre] {
        match self.shelf {
            Shelf::Streaming => self.genres.read(cx).genres(),
            Shelf::Local => &[],
        }
    }
}

impl TableSource for GenreSource {
    type Field = GenreField;

    fn columns(&self) -> &'static [ColumnSpec<GenreField>] {
        COLUMNS
    }

    fn rows(&self, cx: &App) -> usize {
        self.genres(cx).len()
    }

    fn matches(&self, row: usize, query: &str, cx: &App) -> bool {
        self.genres(cx)
            .get(row)
            .is_some_and(|genre| holds(&genre.name, query))
    }

    fn is_loading(&self, cx: &App) -> bool {
        self.shelf == Shelf::Streaming && self.genres.read(cx).is_loading(cx)
    }

    fn picking(&self) -> bool {
        true
    }

    fn cell(&self, cell: Cell<GenreField>, cx: &mut App) -> AnyElement {
        let theme = *cx.theme();
        let muted = theme.muted_foreground;

        let Some(genre) = self.genres(cx).get(cell.row) else {
            return cells::blank(&cell);
        };

        match cell.field {
            GenreField::Cover => cells::artwork(&cell, genre.cover.clone()),
            GenreField::Name => cells::dim(&cell, genre.name.clone(), theme.foreground),
            GenreField::Albums => cells::dim(&cell, genre.albums.to_string(), muted),
            GenreField::Songs => cells::dim(&cell, genre.songs.to_string(), muted),
            GenreField::Index => cells::dim(&cell, (cell.row + 1).to_string(), muted),
        }
    }

    fn compare(&self, field: GenreField, a: usize, b: usize, cx: &App) -> Ordering {
        let genres = self.genres(cx);
        let at = |index: usize| genres.get(index);

        match field {
            GenreField::Name => folded(
                at(a).map(|genre| genre.name.as_str()).unwrap_or_default(),
                at(b).map(|genre| genre.name.as_str()).unwrap_or_default(),
            ),
            GenreField::Albums => at(a)
                .map(|genre| genre.albums)
                .cmp(&at(b).map(|genre| genre.albums)),
            GenreField::Songs => at(a)
                .map(|genre| genre.songs)
                .cmp(&at(b).map(|genre| genre.songs)),
            GenreField::Index | GenreField::Cover => a.cmp(&b),
        }
    }

    fn group(&self, field: GenreField, row: usize, cx: &App) -> Option<SharedString> {
        let genre = self.genres(cx).get(row)?;

        match field {
            GenreField::Name => Some(initial(&genre.name)),
            _ => None,
        }
    }
}
