use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::{Arc, RwLock};

use crate::{AsyncKey, AsyncMap, AsyncStorable, FactoryBorrow};

#[derive(Clone)]
pub struct LockingMap<K, V>
where
    K: 'static + Clone + Hash + Eq + Send + Unpin,
    V: 'static + Clone + Send + Unpin,
{
    map: Arc<RwLock<HashMap<K, V>>>,
}

impl<K: AsyncKey + Sync, V: AsyncStorable + Sync> AsyncMap for LockingMap<K, V> {
    type Key = K;
    type Value = V;

    fn get_if_present(&self, key: &Self::Key) -> Option<Self::Value> {
        match self.map.read() {
            Ok(read_guard) => read_guard.get(key).map(Self::Value::clone),
            Err(_) => {
                panic!("Can't deal with this yet")
            }
        }
    }

    fn get<'a, 'b, B: FactoryBorrow<K, V>>(
        &self,
        key: &'a Self::Key,
        factory: B,
    ) -> Pin<Box<dyn Future<Output = Self::Value> + Send + 'b>> {
        let map = self.map.clone();
        let key = key.clone();
        Box::pin(async move {
            match map.read() {
                Ok(read_guard) => match read_guard.get(&key) {
                    Some(value_ref) => value_ref.clone(),
                    None => {
                        drop(read_guard);
                        LockingMap::create_if_necessary(&map, key, factory)
                    }
                },
                Err(_) => {
                    panic!("Can't deal with this yet");
                }
            }
        })
    }
}

impl<K, V> LockingMap<K, V>
where
    K: AsyncKey,
    V: AsyncStorable,
{
    pub fn new() -> Self {
        LockingMap {
            map: Arc::default(),
        }
    }

    fn create_if_necessary<'a, B: FactoryBorrow<K, V>>(
        map: &'a Arc<RwLock<HashMap<K, V>>>,
        key: K,
        factory: B,
    ) -> V {
        match map.write() {
            Ok(mut map) => match map.get(&key) {
                Some(value_ref) => value_ref.clone(),
                None => {
                    let value = factory.borrow()(&key);
                    map.insert(key, value.clone());
                    value
                }
            },
            Err(_) => {
                todo!()
            }
        }
    }
}
