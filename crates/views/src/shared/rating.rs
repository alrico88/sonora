use gpui::prelude::*;
use gpui::{AnyElement, App, Pixels, SharedString, div, px};
use music::Track;
use state::Sonora;
use ui::{ActiveTheme as _, Button};

const STARS: u8 = 5;
const SIZE: Pixels = px(18.);

/// The five clickable stars that set a track's rating, `seed` keeping the buttons unique within
/// the window. Clicking the star already lit clears the rating.
pub(crate) fn stars(track: &Track, seed: &str, cx: &App) -> AnyElement {
    let theme = *cx.theme();
    let library = Sonora::global(cx).library.clone();
    let id = track.id.as_deref();
    let rating = library.read(cx).rating_of(track);
    let pending = id.is_some_and(|id| library.read(cx).rating_pending(id));

    let mut row = div().flex().flex_none().items_center().gap_0p5();
    for star in 1..=STARS {
        let held = track.clone();
        let library = library.clone();
        let lit = star <= rating;
        row = row.child(
            Button::new(SharedString::from(format!("rating-{seed}-{star}")))
                .ghost()
                .backgroundless()
                .px_0()
                .w(SIZE)
                .h(SIZE)
                .icon(match lit {
                    true => "icons/star-filled.svg",
                    false => "icons/star.svg",
                })
                .tint(match lit {
                    true => theme.primary,
                    false => theme.muted_foreground,
                })
                .disabled(pending || id.is_none())
                .on_click(move |_, _, cx| {
                    let value = match rating == star {
                        true => 0,
                        false => star,
                    };
                    library.update(cx, |library, cx| {
                        library.set_rating(held.clone(), value, cx)
                    });
                }),
        );
    }

    row.into_any_element()
}
