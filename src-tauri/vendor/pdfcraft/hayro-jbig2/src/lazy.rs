#[cfg(feature = "std")]
#[derive(Debug)]
pub(crate) struct Lazy<T>(std::cell::OnceCell<T>);

#[cfg(feature = "std")]
impl<T> Lazy<T> {
    pub(crate) fn new(_value: impl FnOnce() -> T) -> Self {
        Self(std::cell::OnceCell::new())
    }

    pub(crate) fn get(&self, init: impl FnOnce() -> T) -> &T {
        self.0.get_or_init(init)
    }
}

#[cfg(not(feature = "std"))]
#[derive(Debug)]
pub(crate) struct Lazy<T>(T);

#[cfg(not(feature = "std"))]
impl<T> Lazy<T> {
    pub(crate) fn new(value: impl FnOnce() -> T) -> Self {
        Self(value())
    }

    pub(crate) fn get(&self, _init: impl FnOnce() -> T) -> &T {
        &self.0
    }
}
