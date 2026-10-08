use crate::Destination;

pub fn destination(link: &str) -> Option<Destination> {
    let rest = link.trim().strip_prefix("sonora://")?;
    let mut parts = rest.split('/').filter(|part| !part.is_empty());
    let kind = parts.next()?;
    let id = parts.next()?.split(['?', '#']).next()?;
    match kind {
        "track" => Some(Destination::Song(id.to_owned().into())),
        "album" => Some(Destination::Album(id.to_owned().into())),
        "playlist" => Some(Destination::Playlist(id.to_owned().into())),
        "artist" => Some(Destination::Artist(id.to_owned().into())),
        _ => None,
    }
}
