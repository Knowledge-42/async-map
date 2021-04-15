use std::collections::HashMap;
use std::future::{ready, Future};
use std::hash::Hash;
use std::sync::{Arc, RwLock};

use futures::future::FutureExt;

#[derive(Clone)]
pub struct LockingMap<K, V>
where
    K: 'static + Clone + Hash + Eq + Sync + Send + Unpin,
    V: 'static + Clone + Sync + Send + Unpin,
{
    map: Arc<RwLock<HashMap<K, V>>>,
}

impl<K, V> LockingMap<K, V>
where
    K: 'static + Clone + Hash + Eq + Sync + Send + Unpin,
    V: 'static + Clone + Sync + Send + Unpin,
{
    pub fn new() -> Self {
        LockingMap {
            map: Arc::default(),
        }
    }

    pub fn create_if_necessary<'a>(
        map: &'a Arc<RwLock<HashMap<K, V>>>,
        key: &'a K,
        factory: Box<dyn Fn(&K) -> V + Send + 'static>,
    ) -> V {
        match map.write() {
            Ok(mut map) => match map.get(key) {
                Some(value_ref) => value_ref.clone(),
                None => {
                    let value = factory(key);
                    map.insert(key.clone(), value.clone());
                    value
                }
            },
            Err(x) => {
                panic!("Can't deal with this yet");
            }
        }
    }

    pub fn get<'a>(
        &self,
        key: &'a K,
        factory: Box<dyn Fn(&K) -> V + Send + 'static>,
    ) -> impl Future<Output = V> + 'a {
        let map = self.map.clone();
        async move {
            match map.read() {
                Ok(read_guard) => match read_guard.get(key) {
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
        }
    }
}
