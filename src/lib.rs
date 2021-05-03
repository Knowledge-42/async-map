#![feature(trait_alias)]
pub mod lockingmap;
mod versioned_map;

use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;

pub use versioned_map::VersionedMap;

pub trait KeyTrait = Clone + Hash + Eq + Sync + Send + Unpin + 'static;
pub trait ValueTrait = Clone + Sync + Send + Unpin + 'static;

pub trait AsyncMap: Clone + Send {
    type Key: KeyTrait;
    type Value: ValueTrait;

    fn get_if_present(&self, key: &Self::Key) -> Option<Self::Value>;

    fn get<'a>(
        &self,
        key: &'a Self::Key,
        factory: Box<dyn Fn(&Self::Key) -> Self::Value + Send + 'static>,
    ) -> Pin<Box<dyn Future<Output = Self::Value> + Send + 'a>>;
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
