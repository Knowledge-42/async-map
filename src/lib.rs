#![feature(trait_alias)]
pub mod lockingmap;
mod versioned_map;

use std::borrow::Borrow;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;

pub use versioned_map::VersionedMap;

pub trait KeyTrait = Clone + Hash + Eq + Sync + Send + Unpin + 'static;
pub trait ValueTrait = Clone + Send + Sync + Unpin + 'static;

pub trait Factory<K: KeyTrait, V: ValueTrait> = Fn(&K) -> V + Send + Sync;
pub trait FactoryBorrow<K: KeyTrait, V: ValueTrait> =
    Borrow<dyn Factory<K, V>> + Send + 'static + Unpin;

pub trait AsyncMap: Clone + Send {
    type Key: KeyTrait;
    type Value: ValueTrait;

    fn get_if_present(&self, key: &Self::Key) -> Option<Self::Value>;

    fn get<'a, 'b, B: FactoryBorrow<Self::Key, Self::Value>>(
        &'a self,
        key: &'a Self::Key,
        factory: B,
    ) -> Pin<Box<dyn Future<Output = Self::Value> + Send + 'b>>;
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}
