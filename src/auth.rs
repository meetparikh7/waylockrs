use std::thread;

use pam_client::conv_mock::Conversation;
use pam_client::{Context, Flag};
use secstr::SecVec;
use smithay_client_toolkit::reexports::{calloop::EventLoop, calloop::channel};
use users::get_current_username;

use crate::constants;

pub struct PasswordBuffer(SecVec<u8>);

impl PasswordBuffer {
    pub fn new() -> Self {
        Self(SecVec::new(Vec::new()))
    }

    fn zeroize_string(mut data: String) {
        use std::sync::atomic;

        let default = u8::default();

        for c in unsafe { data.as_bytes_mut() } {
            unsafe { std::ptr::write_volatile(c, default) };
        }

        atomic::fence(atomic::Ordering::SeqCst);
        atomic::compiler_fence(atomic::Ordering::SeqCst);
    }

    pub fn append(&mut self, data: String) {
        let bytes = data.as_bytes();
        let mut og_len = self.0.unsecure().len();
        self.0.resize(og_len + bytes.len(), 0);
        for b in bytes {
            self.0.unsecure_mut()[og_len] = *b;
            og_len += 1;
        }
        Self::zeroize_string(data);
    }

    pub fn backspace(&mut self) {
        let og_len = self.0.unsecure().len();
        if og_len != 0 {
            self.0.resize(og_len - 1, 0);
        }
    }

    pub fn unsecure(&self) -> &str {
        unsafe { std::str::from_utf8_unchecked(self.0.unsecure()) }
    }

    pub fn take(&mut self) -> Self {
        let mut new_buffer = SecVec::new(Vec::new());
        std::mem::swap(&mut self.0, &mut new_buffer);
        Self(new_buffer)
    }
}

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

pub fn create_and_run_auth_loop() -> (channel::Sender<PasswordBuffer>, channel::Channel<bool>) {
    struct AuthLoopState {
        auth_res_send: channel::Sender<bool>,
        main_closed: bool,
    }

    let (auth_req_send, auth_req_recv) = channel::channel::<PasswordBuffer>();
    let (auth_res_send, auth_res_recv) = channel::channel::<bool>();

    thread::spawn(move || {
        let mut event_loop: EventLoop<AuthLoopState> = EventLoop::try_new().unwrap();
        event_loop
            .handle()
            .insert_source(auth_req_recv, |evt, _metadata, state| match evt {
                channel::Event::Msg(password) => {
                    let status = verify(password.unsecure());
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
