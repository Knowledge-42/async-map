#![macro_use]
pub mod lockingmap;
mod versioned_map;

use std::borrow::Borrow;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;

pub use versioned_map::VersionedMap;

pub trait KeyTrait: Clone + Hash + Eq + Sync + Send + Unpin + 'static {}
impl<T: Clone + Hash + Eq + Sync + Send + Unpin + 'static> KeyTrait for T {}

pub trait ValueTrait: Clone + Send + Sync + Unpin + 'static {}
impl<T: Clone + Send + Sync + Unpin + 'static> ValueTrait for T {}

pub trait Factory<K: KeyTrait, V: ValueTrait>: Fn(&K) -> V + Send + Sync {}
impl<K: KeyTrait, V: ValueTrait, T: Fn(&K) -> V + Send + Sync> Factory<K, V> for T {}

pub trait FactoryBorrow<K: KeyTrait, V: ValueTrait>:
    Borrow<dyn Factory<K, V>> + Send + 'static + Unpin
{
}
impl<K: KeyTrait, V: ValueTrait, T: Borrow<dyn Factory<K, V>> + Send + 'static + Unpin>
    FactoryBorrow<K, V> for T
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
