//! The mascot: a pixel-art kitten drawn with half blocks, two pixels per cell.

use std::time::Duration;

use ratatui::style::{Color, Style, Stylize};
use ratatui::text::{Line, Span};

pub const WIDTH: u16 = 19;
pub const HEIGHT: u16 = 6;

type Sprite = [&'static str; 12];

const IDLE: Sprite = [
    ".O..........O......",
    ".OO........OO......",
    ".OPOOOOOOOOPO......",
    ".OFFFFFFFFFFO......",
    "OFFEHFFFFEHFFO.....",
    "OFFEEFFFFEEFFO.....",
    "OFFEEFFFFEEFFO.....",
    "OBBFOFNNFOFBBO.....",
    ".OFFFOFFOFFFO......",
    "..OOFFFFFFOO.......",
    ".OFWWFFFFWWFO......",
    "..OOOOOOOOOO.......",
];
// Head down a pixel, body squashed.
const BREATHE: Sprite = [
    "...................",
    ".O..........O......",
    ".OO........OO......",
    ".OPOOOOOOOOPO......",
    ".OFFFFFFFFFFO......",
    "OFFEHFFFFEHFFO.....",
    "OFFEEFFFFEEFFO.....",
    "OFFEEFFFFEEFFO.....",
    "OBBFOFNNFOFBBO.....",
    ".OFFFOFFOFFFO......",
    ".OFWWFFFFWWFO......",
    "..OOOOOOOOOO.......",
];
// Happy ^ ^ eyes, used for blinks too.
const SQUINT: Sprite = [
    ".O..........O......",
    ".OO........OO......",
    ".OPOOOOOOOOPO......",
    ".OFFFFFFFFFFO......",
    "OFFFFFFFFFFFFO.....",
    "OFFEEFFFFEEFFO.....",
    "OFEFFEFFEFFEFO.....",
    "OBBFOFNNFOFBBO.....",
    ".OFFFOFFOFFFO......",
    "..OOFFFFFFOO.......",
    ".OFWWFFFFWWFO......",
    "..OOOOOOOOOO.......",
];
const PAW_UP: Sprite = [
    ".O..........O......",
    ".OO........OO......",
    ".OPOOOOOOOOPO.OO...",
    ".OFFFFFFFFFFOOWWO..",
    "OFFEHFFFFEHFFOWWO..",
    "OFFEEFFFFEEFFOOFO..",
    "OFFEEFFFFEEFFOOFO..",
    "OBBFOFNNFOFBBOOFO..",
    ".OFFFOFFOFFFOOFO...",
    "..OOFFFFFFOOOFO....",
    ".OFWWFFFFFFFO......",
    "..OOOOOOOOOO.......",
];
const SWIPE: Sprite = [
    ".O..........O......",
    ".OO........OO......",
    ".OPOOOOOOOOPO......",
    ".OFFFFFFFFFFO......",
    "OFFEHFFFFEHFFO.....",
    "OFFEEFFFFEEFFO.....",
    "OFFEEFFFFEEFFO.....",
    "OBBFOFNNFOFBBO.OOO.",
    ".OFFFOFFOFFFOOFFWWO",
    "..OOFFFFFFOOOFFWWO.",
    ".OFWWFFFFFFFO..OO..",
    "..OOOOOOOOOO.......",
];

#[derive(Debug, PartialEq, Clone, Copy)]
pub enum Pose {
    Idle,
    Breathe,
    Squint,
    PawUp,
    Swipe,
}

const PAW_UP_FOR: Duration = Duration::from_millis(150);
const SWAT_FOR: Duration = Duration::from_millis(400);
const HAPPY_FOR: Duration = Duration::from_millis(900);
pub const PUFF_FOR: Duration = Duration::from_millis(450);

/// `age` is time since startup; `swat` and `happy` are time since the last quit was sent and the last process died.
/// While `quitting`, the kitten keeps batting at it: paw up, swipe, a pause to watch, again.
pub fn pose(age: Duration, swat: Option<Duration>, happy: Option<Duration>, quitting: bool) -> Pose {
    let bat = Duration::from_millis(age.as_millis() as u64 % 900);
    match (swat, happy) {
        (Some(s), _) if s < PAW_UP_FOR => Pose::PawUp,
        (Some(s), _) if s < SWAT_FOR => Pose::Swipe,
        (_, Some(h)) if h < HAPPY_FOR => Pose::Squint,
        _ if quitting && bat < PAW_UP_FOR => Pose::PawUp,
        _ if quitting && bat < SWAT_FOR => Pose::Swipe,
        _ if quitting => Pose::Idle,
        // Poll is 250ms when idle, so a blink has to outlast it to be seen.
        _ if age.as_millis() % 4000 >= 3700 => Pose::Squint,
        _ if (age.as_millis() / 800) % 2 == 1 => Pose::Breathe,
        _ => Pose::Idle,
    }
}

/// True while a swat or happy squint is playing, so the loop should redraw quickly. Batting is covered by the caller.
pub fn busy(swat: Option<Duration>, happy: Option<Duration>) -> bool {
    swat.is_some_and(|s| s < SWAT_FOR) || happy.is_some_and(|h| h < HAPPY_FOR)
}

/// A just-gone row breaking into dust, sparser every 150ms. None once it has settled.
pub fn puff(line: &Line, age: Duration) -> Option<Line<'static>> {
    if age >= PUFF_FOR {
        return None;
    }
    let stage = (age.as_millis() / 150) as usize;
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    let dust: String = text
        .chars()
        .enumerate()
        .map(|(i, c)| if c == ' ' || (i * 7 + stage * 3) % (stage + 2) != 0 { ' ' } else { ['∘', '°', '·'][(i + stage) % 3] })
        .collect();
    Some(Line::from(dust).dim())
}

fn color(c: u8) -> Option<Color> {
    let (r, g, b) = match c {
        b'O' => (92, 52, 34),    // outline
        b'F' => (244, 164, 84),  // ginger fur
        b'W' => (255, 246, 232), // paws
        b'E' => (40, 28, 28),    // eyes
        b'H' => (255, 255, 255), // eye shine
        b'P' => (255, 158, 178), // inner ears
        b'B' => (255, 186, 186), // blush
        b'N' => (232, 96, 128),  // nose
        _ => return None,
    };
    Some(Color::Rgb(r, g, b))
}

pub fn lines(pose: Pose) -> Vec<Line<'static>> {
    let sprite = match pose {
        Pose::Idle => &IDLE,
        Pose::Breathe => &BREATHE,
        Pose::Squint => &SQUINT,
        Pose::PawUp => &PAW_UP,
        Pose::Swipe => &SWIPE,
    };
    sprite
        .chunks(2)
        .map(|pair| {
            let (top, bottom) = (pair[0].as_bytes(), pair[1].as_bytes());
            Line::from(
                top.iter()
                    .zip(bottom)
                    .map(|(&t, &b)| match (color(t), color(b)) {
                        (Some(t), Some(b)) => Span::styled("▀", Style::new().fg(t).bg(b)),
                        (Some(t), None) => Span::styled("▀", Style::new().fg(t)),
                        (None, Some(b)) => Span::styled("▄", Style::new().fg(b)),
                        (None, None) => Span::raw(" "),
                    })
                    .collect::<Vec<_>>(),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    #[test]
    fn sprites_fit_the_box() {
        for s in [&IDLE, &BREATHE, &SQUINT, &PAW_UP, &SWIPE] {
            assert!(s.iter().all(|r| r.len() == WIDTH as usize));
        }
        assert_eq!(lines(Pose::Idle).len(), HEIGHT as usize);
    }

    #[test]
    fn swat_then_squint_then_idle() {
        assert_eq!(pose(ms(0), Some(ms(50)), None, false), Pose::PawUp);
        assert_eq!(pose(ms(0), Some(ms(200)), Some(ms(0)), false), Pose::Swipe);
        assert_eq!(pose(ms(0), Some(ms(500)), Some(ms(100)), false), Pose::Squint);
        assert_eq!(pose(ms(0), Some(ms(500)), Some(ms(1000)), false), Pose::Idle);
        assert!(busy(Some(ms(300)), None));
        assert!(!busy(Some(ms(500)), Some(ms(1000))));
    }

    #[test]
    fn idle_breathes_and_blinks() {
        assert_eq!(pose(ms(100), None, None, false), Pose::Idle);
        assert_eq!(pose(ms(900), None, None, false), Pose::Breathe);
        assert_eq!(pose(ms(3800), None, None, false), Pose::Squint);
    }

    #[test]
    fn bats_while_quitting_until_happy() {
        assert_eq!(pose(ms(1000), None, None, true), Pose::PawUp);
        assert_eq!(pose(ms(1100), None, None, true), Pose::Swipe);
        assert_eq!(pose(ms(1500), None, None, true), Pose::Idle);
        assert_eq!(pose(ms(1000), None, Some(ms(10)), true), Pose::Squint);
    }

    #[test]
    fn puff_thins_out_then_settles() {
        let row = Line::from("● Slack            412 MB");
        let count = |age| puff(&row, ms(age)).unwrap().to_string().chars().filter(|c| *c != ' ').count();
        assert!(count(0) > count(200) && count(200) > 0);
        assert!(puff(&row, ms(500)).is_none());
    }
}
