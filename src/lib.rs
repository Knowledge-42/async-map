//! This crate provides data-structure for concurrent use by many tasks in an asynschronous contexts,
//! with a particular focus on high-read/low-write situations.
#![crate_name = "async_map"]
#![macro_use]
#[doc(hidden)] // Really just to provide a comparison for bechmarking
pub mod lockingmap;
pub mod non_locking_map;
pub mod single_writer_versioned;
mod versioned_map;

use std::borrow::Borrow;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;

pub use versioned_map::VersionedMap;

/// A trait for types that can be held in a collection used in an asynchronous context,
/// which might be shared between many tasks. A blanket implementation is provided.
pub trait AsyncStorable: Clone + Send + Sync + std::fmt::Debug + Unpin + 'static {}
impl<T: Clone + Send + Sync + Unpin + std::fmt::Debug + 'static> AsyncStorable for T {}

/// A trait for types that can be keys in an asynchronous map. A blanket implementation is provided.
pub trait AsyncKey: AsyncStorable + Hash + Eq {}
impl<T: AsyncStorable + Hash + Eq> AsyncKey for T {}

/// A trait for factory methods that can be used to create new values for a key in an asynchronous map. A blanket implementation is provided.
pub trait AsyncFactory<K: AsyncKey, V: AsyncStorable>:
    (Fn(&K) -> V) + Send + Sync + 'static
{
}

impl<K: AsyncKey, V: AsyncStorable, F: (Fn(&K) -> V) + Send + Sync + 'static> AsyncFactory<K, V>
    for F
{
}

/// A trait for types from which a factory method can be borrowed. A blanket implementation is provided.
pub trait FactoryBorrow<K: AsyncKey, V: AsyncStorable>:
    Borrow<dyn AsyncFactory<K, V>> + Send + Unpin + 'static
{
}

impl<K: AsyncKey, V: AsyncStorable, T: Borrow<dyn AsyncFactory<K, V>> + Send + Unpin + 'static>
    FactoryBorrow<K, V> for T
{
}

pub trait AsyncMap: Clone + Send {
    type Key: AsyncKey;
    type Value: AsyncStorable;
    fn get_if_present(&self, key: &Self::Key) -> Option<Self::Value>;

    fn get<'a, 'b, B: FactoryBorrow<Self::Key, Self::Value>>(
        &'a self,
        key: &'a Self::Key,
        factory: B,
    ) -> Pin<Box<dyn Future<Output = Self::Value> + Send + 'b>>;
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::{AsyncFactory, AsyncKey, AsyncStorable, FactoryBorrow};

    #[derive(Clone, PartialEq, Eq, Hash, Debug)]
    pub struct TestKey(pub String);

    #[derive(Clone, PartialEq, Debug)]
    pub struct TestValue(pub String);

    fn print_value<T: AsyncStorable>(value: T) {
        println!("value: {:?}", value);
    }

    fn factory(key: &TestKey) -> TestValue {
        TestValue(key.0.clone())
    }

    fn call_factory2<K: AsyncKey, V: AsyncStorable, F: FactoryBorrow<K, V>>(fact: F, key: &K) -> V {
        fact.borrow()(key)
    }

    #[test]
    fn it_works() {
        let key = TestKey("test".to_string());
        let value = TestValue("test value".to_string());

        let factory = Arc::new(factory);

        print_value(value);

        // Not testing anything, this test demonstrates compilation
        call_factory2(factory as Arc<dyn AsyncFactory<TestKey, TestValue>>, &key);
    }
}
