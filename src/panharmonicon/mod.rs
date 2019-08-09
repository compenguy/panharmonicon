use cursive::direction::Orientation;
use cursive::view::{ScrollStrategy, SizeConstraint};
use cursive::views::{BoxView, LinearLayout, Panel, ProgressBar, ScrollView, SelectView, TextView};
use cursive::{Cursive, ScreenId};

use log::trace;

use crate::config::Config;
use crate::errors::{Error, Result};

use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct Panharmonicon {
    win: Cursive,
    playing_screen: Option<ScreenId>,
}

impl Panharmonicon {
    pub fn new(_config: Rc<RefCell<Config>>) -> Result<Self> {
        let mut terminal = Self {
            win: Cursive::default(),
            playing_screen: None,
        };

        terminal.init_views();
        Ok(terminal)
    }

    fn init_views(&mut self) {
        if self.playing_screen.is_some() {
            return;
        }
        self.playing_screen = Some(self.win.add_screen());
        let top = LinearLayout::new(Orientation::Vertical)
            .child(
                LinearLayout::new(Orientation::Horizontal)
                    .child(
                        BoxView::new(
                            SizeConstraint::Free,
                            SizeConstraint::Full,
                            Panel::new(
                                ScrollView::new({
                                    let mut v = SelectView::new();
                                    v.add_item("", 0);
                                    v
                                })
                                .scroll_strategy(ScrollStrategy::StickToBottom),
                            )
                            .title("Playlist"),
                        )
                        .squishable(),
                    )
                    .child(
                        BoxView::new(
                            SizeConstraint::Full,
                            SizeConstraint::Full,
                            Panel::new(ScrollView::new(TextView::empty())).title(""),
                        )
                        .squishable(),
                    ),
            )
            .child(
                BoxView::new(
                    SizeConstraint::Full,
                    SizeConstraint::AtLeast(4),
                    Panel::new(ProgressBar::new()).title(""),
                )
                .squishable(),
            );

        self.win.add_fullscreen_layer(top);
    }

    pub fn run(&mut self) {
        self.win.run()
    }
}
