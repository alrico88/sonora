pub mod artwork;
mod audio;
pub mod binimum;
pub mod credentials;
pub mod engine;
pub mod equalizer;
pub mod escape;
pub mod kugou;
pub mod local;
pub mod lrclib;
pub mod lyrics;
mod models;
pub mod musixmatch;
pub mod netease;
pub mod progress;
pub mod scrobble;
mod sink;
mod spectrum;
mod stream;
pub mod subsonic;
pub mod trouble;

use std::collections::HashMap;
use std::num::NonZero;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use anyhow::Result;
use async_trait::async_trait;
use time::format_description::well_known::Iso8601;
use time::parsing::Parsed;
use time::{Month, OffsetDateTime};

pub use equalizer::Equalizer;
pub use models::{
    Album, AlbumCatalogue, AlbumDetail, Artist, ArtistProfile, ArtistRef, Contributor, Credit,
    Genre, GenreDetail, GenreItem, GenreSection, HomeFeed, Lyrics, LyricsHit, LyricsLane,
    LyricsLine, LyricsQuery, LyricsWord, Playlist, PlaylistDetail, ReleaseType, RomanizedText,
    SavedArtist, Track, TrackKey, TrackTags, UserProfile, Voice, WritingSystem,
};
pub use spectrum::Spectrum;

pub const LOCAL_TRACK_PREFIX: &str = "local:";
pub const LOCAL_ALBUM_PREFIX: &str = "local-album:";
pub const LOCAL_ARTIST_PREFIX: &str = "local-artist:";
pub const LOCAL_PLAYLIST_PREFIX: &str = "local-playlist:";

/// The most recommendations a provider hands one list of an album or artist page, so a
/// rail never asks for or draws more than this many releases or artists.
pub const SUGGESTIONS: usize = 10;

pub fn is_local_id(id: &str) -> bool {
    id.starts_with(LOCAL_TRACK_PREFIX)
        || id.starts_with(LOCAL_ALBUM_PREFIX)
        || id.starts_with(LOCAL_ARTIST_PREFIX)
        || id.starts_with(LOCAL_PLAYLIST_PREFIX)
}

pub fn distinct_covers(tracks: &[Track], wanted: usize) -> Vec<String> {
    let mut covers: Vec<String> = Vec::with_capacity(wanted);
    for cover in tracks.iter().filter_map(|track| track.cover.as_deref()) {
        if covers.len() == wanted {
            break;
        }
        if !covers.iter().any(|kept| kept == cover) {
            covers.push(cover.to_owned());
        }
    }

    covers
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MediaKind {
    Track,
    Album,
    Artist,
    Playlist,
}

/// What `MusicApi::report` tells the provider's server about the current track.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Report {
    Playing,
    Paused,
    Stopped,
}

#[async_trait]
pub trait MusicApi: Send + Sync {
    fn share_url(&self, kind: MediaKind, id: &str) -> Option<String>;
    async fn profile(&self) -> Result<UserProfile>;

    async fn artist(&self, artist_id: &str) -> Result<Artist>;

    async fn artist_profile(&self, artist_id: &str) -> Result<ArtistProfile>;
    async fn artist_images(&self, ids: Vec<String>) -> Result<HashMap<String, String>>;

    /// The tracks the user starred. On a `Shape::Saved` provider this is the whole songs
    /// library; on a `Shape::Catalog` one it only feeds the hearts and the favorites filter.
    async fn saved_tracks(&self) -> Result<Vec<Track>>;

    /// Every track the provider has. Only a `Shape::Catalog` provider answers.
    async fn all_tracks(&self) -> Result<Vec<Track>> {
        Ok(Vec::new())
    }

    async fn set_track_saved(&self, track_id: &str, saved: bool) -> Result<()>;

    /// Sets the listener's own 1–5 star rating for a track, `0` clearing it. A provider that
    /// keeps no rating of its own keeps the default and makes no request.
    async fn set_track_rating(&self, _track_id: &str, _rating: u8) -> Result<()> {
        anyhow::bail!("this provider does not support ratings")
    }

    /// Reads what the file itself says, for a provider whose tracks are files.
    async fn track_tags(&self, _track_id: &str) -> Result<TrackTags> {
        anyhow::bail!("this provider cannot edit tags")
    }

    async fn set_track_tags(&self, _track_id: &str, _tags: TrackTags) -> Result<()> {
        anyhow::bail!("this provider cannot edit tags")
    }
    async fn track(&self, track_id: &str) -> Result<Track>;

    /// Delete a track file from disk (only for local provider)
    async fn delete_track_file(&self, _track_id: &str) -> Result<()> {
        anyhow::bail!("this provider does not support file deletion")
    }
    async fn track_playcount(&self, track_id: &str) -> Result<Option<u64>>;

    /// Tells the provider's own server whether a track is playing and where it is. It is sent on
    /// every start, pause, seek and stop, and never counts as a listen. A provider that keeps no
    /// listening record keeps the default and makes no request.
    async fn report(&self, _track_id: &str, _report: Report, _position: Duration) -> Result<()> {
        Ok(())
    }

    /// Records a finished listen that started at `at` on the provider's own server. It is sent
    /// at the same moment and under the same rules as a scrobble. A provider that keeps no
    /// listening record keeps the default and makes no request.
    async fn played(&self, _track_id: &str, _at: SystemTime) -> Result<()> {
        Ok(())
    }

    async fn playlists(&self) -> Result<Vec<Playlist>>;
    async fn create_playlist(&self, name: &str) -> Result<String>;
    async fn rename_playlist(&self, playlist_id: &str, name: &str) -> Result<()>;
    async fn delete_playlist(&self, playlist_id: &str) -> Result<()>;
    async fn remove_playlist_from_library(&self, playlist_id: &str) -> Result<()>;
    async fn add_playlist_to_library(&self, playlist_id: &str) -> Result<()>;

    async fn set_playlist_public(&self, playlist_id: &str, public: bool) -> Result<()>;
    async fn add_track_to_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()>;
    async fn remove_track_from_playlist(&self, playlist_id: &str, track_id: &str) -> Result<()>;
    async fn saved_albums(&self) -> Result<Vec<Album>>;

    /// Every album the provider has. Only a `Shape::Catalog` provider answers.
    async fn all_albums(&self) -> Result<Vec<Album>> {
        Ok(Vec::new())
    }

    async fn set_album_saved(&self, album_id: &str, saved: bool) -> Result<()>;
    async fn saved_artists(&self) -> Result<Vec<SavedArtist>>;

    /// Every artist the provider has. Only a `Shape::Catalog` provider answers.
    async fn all_artists(&self) -> Result<Vec<SavedArtist>> {
        Ok(Vec::new())
    }

    async fn set_artist_saved(&self, artist_id: &str, saved: bool) -> Result<()>;
    async fn album(&self, album_id: &str) -> Result<AlbumDetail>;
    async fn album_tracks(&self, album_id: &str) -> Result<Vec<Track>>;

    /// The rest of an album page, fetched once `album` has put the tracks up: the releases
    /// the provider lists as related, with more from the same artist first and similar
    /// artists' releases topping the rail up. `artist_id` is
    /// the page's artist when the album names one the app can follow. A provider whose
    /// `album` already answers with everything leaves the default, which is nothing more
    /// to fetch.
    async fn album_catalogue(
        &self,
        _album_id: &str,
        _artist_id: Option<&str>,
    ) -> Result<AlbumCatalogue> {
        Ok(AlbumCatalogue::default())
    }
    async fn playlist(&self, playlist_id: &str) -> Result<PlaylistDetail>;
    async fn playlist_tracks(&self, playlist_id: &str) -> Result<Vec<Track>>;
    async fn playlist_covers(&self, playlist_id: &str, wanted: usize) -> Result<Vec<String>>;
    /// The station seeded by `track_id`, from its start or from `from`, a continuation an
    /// earlier call answered with. The continuation that comes back fetches the next stretch,
    /// and `None` means the provider has no more. A provider that serves a station in one go
    /// ignores `from` and answers `None`, which it is then never handed.
    async fn track_radio(
        &self,
        track_id: &str,
        from: Option<&str>,
    ) -> Result<(Vec<Track>, Option<String>)>;

    async fn search(&self, query: &str) -> Result<Vec<Track>>;

    async fn search_albums(&self, _query: &str) -> Result<Vec<Album>> {
        Ok(Vec::new())
    }

    async fn search_playlists(&self, _query: &str) -> Result<Vec<Playlist>> {
        Ok(Vec::new())
    }

    async fn home(&self) -> Result<HomeFeed> {
        Ok(HomeFeed::default())
    }

    async fn genres(&self) -> Result<Vec<Genre>> {
        Ok(Vec::new())
    }

    async fn genre(&self, _genre_id: &str) -> Result<GenreDetail> {
        Ok(GenreDetail::default())
    }
}

#[async_trait]
pub trait LyricsProvider: Send + Sync {
    fn name(&self) -> &'static str;
    async fn search(&self, query: &LyricsQuery) -> Result<Vec<LyricsHit>>;
}

/// What an engine is started with. `equalizer` is shared rather than copied: the engine keeps
/// reading it, so a change reaches the output without a restart.
#[derive(Clone, Debug)]
pub struct PlaybackConfig {
    pub normalisation: bool,
    pub gapless: bool,
    pub position_interval: Duration,
    pub gain: f32,
    pub equalizer: Equalizer,
}

/// What an engine reports back. `Playing` and `Seeked` mean audio from `at` is reaching the
/// output, not that a decoder is ready, so whatever follows the sound can start on them.
#[derive(Clone, Debug, PartialEq)]
pub enum PlaybackEvent {
    Loading {
        id: Option<String>,
        at: Duration,
    },
    Playing {
        id: Option<String>,
        at: Duration,
    },
    Paused {
        id: Option<String>,
        at: Duration,
    },
    /// A progress report from the decoder. It runs ahead of what is audible by whatever the
    /// output has queued, and none arrive while the engine is busy with a seek.
    Position {
        id: Option<String>,
        at: Duration,
    },
    /// Audio from the new position has reached the output after a seek.
    Seeked {
        id: Option<String>,
        at: Duration,
    },
    Length {
        id: Option<String>,
        duration: Duration,
    },
    /// The whole of the current track has arrived, so fetching the next one takes nothing from
    /// it. Engines that cannot tell never send this.
    Downloaded {
        id: Option<String>,
    },
    Ended {
        id: Option<String>,
    },
    Unavailable {
        id: Option<String>,
    },
    /// The provider turned the load down for now, as it does under a rate limit, so the same
    /// track may play after a wait.
    Throttled {
        id: Option<String>,
    },
    OutputChanged,
}

impl PlaybackEvent {
    pub fn id(&self) -> Option<&str> {
        match self {
            Self::Loading { id, .. }
            | Self::Playing { id, .. }
            | Self::Paused { id, .. }
            | Self::Position { id, .. }
            | Self::Seeked { id, .. }
            | Self::Length { id, .. }
            | Self::Downloaded { id }
            | Self::Ended { id, .. }
            | Self::Unavailable { id, .. }
            | Self::Throttled { id } => id.as_deref(),
            _ => None,
        }
    }
}

/// Transport control of one engine. Every call is fire-and-forget: the outcome arrives as a
/// `PlaybackEvent`, never as a return value.
pub trait Player: Send + Sync {
    /// Fetches a track and plays it from `at`. A `seamless` load is a queue segue: a gapless
    /// engine keeps what it has queued so the join has no gap, any other load drops it.
    fn load(&self, track_id: &str, at: Duration, seamless: bool) -> Result<()>;

    /// Fetches a track and leaves it paused at `at`, ready for `play`.
    fn load_paused_at(&self, track_id: &str, at: Duration) -> Result<()>;

    /// Fetches a track ahead of time so a later `load` starts at once. `segue` marks the next
    /// queue item, which a gapless engine may already line up behind the current one.
    fn preload(&self, track_id: &str, segue: bool) -> Result<()>;
    fn play(&self);
    fn pause(&self);

    /// Moves to `position`. While loading, the track starts there instead; while playing, a
    /// `Seeked` event follows once audio from there reaches the output.
    fn seek(&self, position: Duration);
    fn set_gain(&self, gain: f32);

    fn spectrum(&self) -> Option<Spectrum> {
        None
    }
}

#[async_trait]
pub trait PlaybackEvents: Send {
    async fn next(&mut self) -> Option<PlaybackEvent>;
}

pub trait PlaybackFactory: Send + Sync {
    fn start(&self, config: PlaybackConfig) -> (Box<dyn Player>, Box<dyn PlaybackEvents>);
}

/// What a provider's library is made of. It decides which `MusicApi` methods fill the library
/// pages and whether a favorites filter is offered on them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Shape {
    /// The library is what the user starred, read through the `saved_*` methods.
    Saved,
    /// The library is everything the provider has, read through the `all_*` methods, with the
    /// `saved_*` set drawn on top as hearts and a filter.
    Catalog,
}

/// What a provider can do beyond listing and playing, so a control it has no answer for is
/// never put in front of the listener.
///
/// This is about the service, not the account: something a provider simply does not have. A capability that
/// is off hides its button, its menu item and its column, rather than showing one that fails
/// when pressed. A provider that gains one flips a flag here and the UI follows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    /// The listener can follow and unfollow an artist.
    pub follow_artists: bool,
    /// A track can seed a station, which is what fills the queue behind it.
    pub radio: bool,
    /// Tracks carry a play count worth a column of its own.
    pub playcounts: bool,
    /// The listener can give a track a 1–5 star rating, which a column and the player bar set
    /// through `set_track_rating`.
    pub ratings: bool,
}

impl Capabilities {
    /// What a full streaming service offers.
    pub const ALL: Self = Self {
        follow_artists: true,
        radio: true,
        playcounts: true,
        ratings: false,
    };

    /// Nothing beyond listing and playing.
    pub const NONE: Self = Self {
        follow_artists: false,
        radio: false,
        playcounts: false,
        ratings: false,
    };
}

pub struct ProviderSession {
    pub profile: UserProfile,
    pub api: Arc<dyn MusicApi>,
    pub playback: Arc<dyn PlaybackFactory>,
    pub shape: Shape,
    pub authenticated: bool,
    pub capabilities: Capabilities,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SignIn {
    Path(Vec<PathBuf>),
    Credentials {
        server: String,
        username: String,
        password: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignInProblem {
    Credentials,
    Network,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignInFailure(pub SignInProblem);

impl std::fmt::Display for SignInFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let reason = match self.0 {
            SignInProblem::Credentials => "the credentials are not valid",
            SignInProblem::Network => "the provider could not be reached",
        };
        write!(f, "{reason}")
    }
}

impl std::error::Error for SignInFailure {}

#[async_trait]
pub trait MusicProvider: Send + Sync {
    fn name(&self) -> &'static str;
    fn slug(&self) -> &'static str;

    /// Forgets whatever the provider remembers about its last scan, so the next one reads
    /// everything again. Only a provider that scans files has anything to forget, and only a
    /// rescan the user asked for should ask it to.
    fn forget_scan(&self) {}
    /// A factory for the provider's playback engine that can work without sign-in and scan,
    /// so playback can start before the library has loaded. Only for a provider whose tracks
    /// are files.
    fn playback_factory(&self) -> Option<Arc<dyn PlaybackFactory>> {
        None
    }
    /// Reads an arbitrary file on disk as a track, for a provider whose tracks are files. Used
    /// by file-association opens, which may point outside any scanned folder.
    fn track_from_path(&self, _path: &Path) -> Option<Track> {
        None
    }
    fn sign_in_options(&self) -> Vec<SignIn>;
    fn stored(&self) -> bool;
    fn location(&self) -> Option<String> {
        None
    }
    /// The host to open a connection to when checking whether the network is back. `None`
    /// where the provider needs no network, which is what keeps a local library from ever
    /// looking for one.
    fn reach(&self) -> Option<String> {
        None
    }
    /// What a status calls this provider after "listening to". A service answers with its own
    /// name; one that is only the user's own files says what the files are instead.
    fn listening_to(&self) -> &'static str {
        self.name()
    }
    async fn restore(&self) -> Result<Option<ProviderSession>>;
    async fn sign_in(&self, method: SignIn) -> Result<ProviderSession>;
    fn sign_out(&self);
}

/// Leniently convert an ISO8601 timestamp to unix epoch seconds. Accepts only
/// the date portion, date and time portions, or full date and time with offset.
pub fn iso_8601_to_epoch(value: Option<&str>) -> Option<i64> {
    let time_str = value?.as_bytes();
    let defaults = Parsed::new()
        .with_month(Month::January)
        .and_then(|d| d.with_day(NonZero::<u8>::new(1)?))
        .and_then(|d| d.with_hour_24(0))
        .and_then(|d| d.with_minute(0))
        .and_then(|d| d.with_second(0))
        .and_then(|d| d.with_subsecond(0))
        .and_then(|d| d.with_offset_hour(0))
        .and_then(|d| d.with_offset_minute_signed(0))
        .and_then(|d| d.with_offset_second_signed(0))?;
    let timestamp = OffsetDateTime::parse_with_defaults(time_str, &Iso8601::PARSING, defaults)
        .ok()?
        .unix_timestamp();
    Some(timestamp)
}

#[cfg(test)]
mod tests {
    use time::macros::datetime;

    use super::*;

    #[test]
    fn iso_8601_to_epoch_parses_correctly() {
        // None and malformed input
        assert_eq!(iso_8601_to_epoch(None), None);
        assert_eq!(iso_8601_to_epoch(Some("")), None);
        assert_eq!(iso_8601_to_epoch(Some("malformed")), None);

        // Basic epoch format
        assert_eq!(iso_8601_to_epoch(Some("1970-01-01")), Some(0));
        assert_eq!(iso_8601_to_epoch(Some("1970-01-01T00:00:00")), Some(0));
        assert_eq!(iso_8601_to_epoch(Some("1970-01-01T00:00:00Z")), Some(0));
        assert_eq!(
            iso_8601_to_epoch(Some("1970-01-01T00:00:00+00:00")),
            Some(0)
        );

        // A specific date with/without time/offset
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20")),
            Some(datetime!(2021-03-20 00:00:00 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T13:45:07")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T13:45:07.123456")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T13:45:07Z")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T13:45:07+00:00")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
        assert_eq!(
            iso_8601_to_epoch(Some("2021-03-20T10:45:07-03:00")),
            Some(datetime!(2021-03-20 13:45:07 UTC).unix_timestamp())
        );
    }
}
