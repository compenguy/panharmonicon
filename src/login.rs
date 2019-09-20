use std::cell::RefCell;
use std::rc::Rc;

use crate::config::Config;
use crate::errors::Result;

use cursive::traits::*;
use cursive::views::{Dialog, EditView};
use cursive::Cursive;

pub(crate) fn login_cursive(config: Rc<RefCell<Config>>, win: Rc<RefCell<Cursive>>) -> Result<()> {
    while config.borrow().login.get_username().is_none() {
        request_username(win.clone());
    }
    while config.borrow().login.get_password()?.is_none() {
        request_password(win.clone());
    }
    Ok(())
}

pub(crate) fn request_username(win: Rc<RefCell<Cursive>>) {
    win.borrow_mut().add_layer(
        Dialog::new()
            .title("Pandora Username")
            // Padding is (left, right, top, bottom)
            .padding((1, 1, 1, 0))
            .content(
                EditView::new()
                    .on_submit(|win, u| {
                        win.with_user_data(|config: &mut Rc<RefCell<Config>>| {
                            config.borrow_mut().login.update_username(u);
                        });
                        win.pop_layer();
                        win.quit();
                    })
                    .with_id("login::request_username")
                    // Wrap in a fixed-with BoxView
                    .fixed_width(20),
            ),
    );
    win.borrow_mut().run();
}

pub(crate) fn request_password(win: Rc<RefCell<Cursive>>) {
    win.borrow_mut().add_layer(
        Dialog::new()
            .title("Pandora Password")
            // Padding is (left, right, top, bottom)
            .padding((1, 1, 1, 0))
            .content(
                EditView::new()
                    .secret()
                    .on_submit(|win, p| {
                        win.with_user_data(|config: &mut Rc<RefCell<Config>>| {
                            // TODO: handle error by creating popup
                            let _ = config.borrow_mut().login.update_password(p);
                        });
                        win.pop_layer();
                        win.quit();
                    })
                    .with_id("login::request_password")
                    // Wrap in a fixed-with BoxView
                    .fixed_width(20),
            ),
    );
    win.borrow_mut().run();
}
