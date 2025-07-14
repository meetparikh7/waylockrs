use std::thread;

use pam_client::conv_mock::Conversation;
use pam_client::{Context, Flag};
use smithay_client_toolkit::reexports::{calloop::EventLoop, calloop::channel};
use users::get_current_username;

use crate::constants;

pub fn verify(password: &str) -> bool {
    let username = get_current_username()
        .expect("Failed to get username")
        .to_string_lossy()
        .to_string();
    let conv = Conversation::with_credentials(username.clone(), password);

    let mut context = Context::new(
        constants::SERVICE_NAME, // Service name, decides which policy is used (see `/etc/pam.d`)
        Some(username.as_str()), // Optional preset user name
        conv,                    // Handler for user interaction
    )
    .expect("Failed to initialize PAM context");

    // Authenticate the user (ask for password, 2nd-factor token, fingerprint, etc.)
    return context.authenticate(Flag::NONE).is_ok();
}

pub fn create_and_run_auth_loop() -> (channel::Sender<String>, channel::Channel<bool>) {
    struct AuthLoopState {
        auth_res_send: channel::Sender<bool>,
        main_closed: bool,
    }

    let (auth_req_send, auth_req_recv) = channel::channel::<String>();
    let (auth_res_send, auth_res_recv) = channel::channel::<bool>();

    thread::spawn(move || {
        let mut event_loop: EventLoop<AuthLoopState> = EventLoop::try_new().unwrap();
        event_loop
            .handle()
            .insert_source(auth_req_recv, |evt, _metadata, state| match evt {
                channel::Event::Msg(password) => {
                    let status = verify(&password);
                    state.auth_res_send.send(status).unwrap();
                }
                channel::Event::Closed => state.main_closed = true,
            })
            .unwrap();

        let mut state = AuthLoopState {
            auth_res_send,
            main_closed: false,
        };

        while !state.main_closed {
            event_loop
                .dispatch(None, &mut state)
                .expect("Failed to run");
        }
    });

    (auth_req_send, auth_res_recv)
}
