use pam_client::conv_mock::Conversation;
use pam_client::{Context, Flag};
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
