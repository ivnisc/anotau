mod activity;
mod app;
mod editor;
mod export;
mod inbox;
mod links;
mod markdown;
mod network;
mod palette;
mod people;
mod popup;
mod progress;
mod selection;
mod table;
mod terminal;
mod theme;
mod welcome;
mod workspace;

use ratatui::crossterm::event;
use std::{env, sync::Arc};

pub(crate) enum Update {
    Draw,
    Idle,
    Exit,
    StartEditor,
    Authenticate {
        credentials: anotau_protocol::Credentials,
        account: network::Account,
    },
    SignOut,
    /// Petición que no pasa por la cola de escrituras del editor, como recuperar o cambiar la
    /// contraseña, borrar la cuenta o exportar; espera a que la red quede libre.
    Send(network::Job),
    ToggleMouseCapture,
    OpenLink(Arc<str>),
}

fn main() -> anyhow::Result<()> {
    if env::args_os().len() != 1 {
        anyhow::bail!(
            "Uso: anotau. Configura ANOTAU_SERVER_URL y, si corresponde, ANOTAU_CA_CERT."
        );
    }
    let config = network::Config::from_env()?;
    let address = config.address();
    let network = network::Network::start(config)?;
    let (mut session, mut terminal) = terminal::Session::start()?;
    let mut app = app::App::new(address)?;
    while !app.finished {
        for response in network.responses.try_iter() {
            app.response(response, &mut session)?;
        }
        app.tick(&network, &mut session)?;
        if app.draw {
            terminal.draw(|frame| app.render(frame))?;
        }
        if event::poll(app.wait())? {
            app.input(event::read()?, &mut session)?;
        }
    }
    Ok(())
}
