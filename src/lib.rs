//! This crate provides data-structure for concurrent use by many tasks in an asynschronous contexts,
//! with a particular focus on high-read/low-write situations.
#![crate_name = "async_map"]
#![macro_use]
pub mod lockingmap;
pub mod non_locking_map;
mod single_writer_versioned;
mod versioned_map;

use std::borrow::Borrow;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;

pub use versioned_map::VersionedMap;

pub trait KeyTrait: Clone + Hash + Eq + Sync + Send + Unpin + std::fmt::Debug + 'static {}
impl<T: Clone + Hash + Eq + Sync + Send + Unpin + std::fmt::Debug + 'static> KeyTrait for T {}

pub trait ValueTrait: Clone + Send + Sync + std::fmt::Debug + Unpin + 'static {}
impl<T: Clone + Send + Sync + Unpin + std::fmt::Debug + 'static> ValueTrait for T {}

pub trait FactoryBorrow<K: KeyTrait, V: ValueTrait>:
    Borrow<dyn Fn(&K) -> V + Send + Sync> + Send + 'static + Unpin
{
}
impl<
        K: KeyTrait,
        V: ValueTrait,
        T: Borrow<dyn Fn(&K) -> V + Send + Sync> + Send + 'static + Unpin,
    > FactoryBorrow<K, V> for T
{
}

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
