use cursive::direction::Orientation;
use cursive::traits::*;
use cursive::view::{ScrollStrategy, SizeConstraint};
use cursive::views::{BoxView, LinearLayout, Panel, ProgressBar, ScrollView, SelectView, TextView};
use cursive::{Cursive, ScreenId};

use crate::config::Config;

use std::cell::RefCell;
use std::rc::Rc;

pub(crate) struct Panharmonicon {
    win: Rc<RefCell<Cursive>>,
    playing_screen: Option<ScreenId>,
}

impl Panharmonicon {
    pub fn new(_config: Rc<RefCell<Config>>, win: Rc<RefCell<Cursive>>) -> Self {
        let mut terminal = Self {
            win,
            playing_screen: None,
        };

        terminal.init_views();
        terminal
    }

    fn init_views(&mut self) {
        if self.playing_screen.is_some() {
            return;
        }
        self.playing_screen = Some(self.win.borrow_mut().add_screen());
        let top = LinearLayout::new(Orientation::Vertical)
            .child(
                LinearLayout::new(Orientation::Horizontal)
                    .child(
                        BoxView::new(
                            SizeConstraint::Free,
                            SizeConstraint::Full,
                            Panel::new(
                                ScrollView::new(SelectView::<String>::new().with_id("Playlist"))
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
                            Panel::new(ScrollView::new(TextView::empty().with_id("Info")))
                                .title(""),
                        )
                        .squishable(),
                    ),
            )
            .child(
                BoxView::new(
                    SizeConstraint::Full,
                    SizeConstraint::AtLeast(4),
                    Panel::new(ProgressBar::new().with_id("PlayingProgress"))
                        .title("")
                        .with_id("NowPlaying"),
                )
                .squishable(),
            );

        self.win.borrow_mut().add_fullscreen_layer(top);
        self.win.borrow_mut().add_global_callback('q', |s| s.quit())
    }

    pub fn run(&mut self) {
        self.win.borrow_mut().run()
    }
}
