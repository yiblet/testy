use std::error::Error;

pub trait ToStringResult<S, E> {
    fn to_string_result(self) -> Result<S, String>;
}

impl<S, E: Error> ToStringResult<S, E> for Result<S, E> {
    fn to_string_result(self) -> Result<S, String> {
        self.map_err(|e| e.to_string())
    }
}
