mod auth;
mod constants;

pub fn main() {
    match rpassword::prompt_password("Enter password: ") {
        Err(_) => eprintln!("Failed to get password"),
        Ok(password) => println!("{:?}", auth::verify(password.as_str())),
    };
}
