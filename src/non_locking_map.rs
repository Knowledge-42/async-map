use crate::single_writer_versioned::{DataUpdater, Versioned};
use crate::{AsyncMap, FactoryBorrow, KeyTrait, ValueTrait};
use im;

use futures::{FutureExt, TryFutureExt};
use std::future::{ready, Future};
use std::pin::Pin;

#[derive(Clone)]
pub struct NonLockingMap<K: KeyTrait, V: ValueTrait> {
    versioned: Versioned<im::HashMap<K, V>>,
}

impl<K: KeyTrait, V: ValueTrait> NonLockingMap<K, V> {
    pub fn new() -> NonLockingMap<K, V> {
        NonLockingMap {
            versioned: Versioned::from_initial(im::HashMap::new()).0, // No quitting!
        }
    }
}

impl<K: KeyTrait, V: ValueTrait> AsyncMap for NonLockingMap<K, V> {
    type Key = K;
    type Value = V;
    fn get_if_present(&self, key: &K) -> Option<V> {
        self.versioned
            .with_latest(|map| map.get(key).map(|value| value.clone()))
    }

    fn get<'a, 'b, F: FactoryBorrow<K, V>>(
        &'a self,
        key: &'a K,
        factory: F,
    ) -> Pin<Box<(dyn Future<Output = V> + Send + 'b)>> {
        match self.get_if_present(key) {
            Some(value) => ready(value).boxed(),
            None => {
                let (sender, receiver) = tokio::sync::oneshot::channel();

                let key = key.clone();
                let updater: Box<dyn DataUpdater<im::HashMap<K, V>>> =
                    Box::new(move |map| match map.get(&key) {
                        Some(value) => {
                            sender.send(value.clone()).expect("Send failed!");
                            None
                        }
                        None => {
                            let new_value = (*factory.borrow())(&key);
                            let new_map = map.update(key, new_value.clone());
                            sender.send(new_value).expect("Send failed!");
                            Some(new_map)
                        }
                    });

                if self.versioned.clone().update(updater).is_err() {
                    panic!("Update failed");
                }

                receiver
                    .unwrap_or_else(|_| panic!("Oneshot receive failed!"))
                    .boxed()
            }
        }
    }
}

#[cfg(test)]
mod test {

    use super::NonLockingMap as VersionedMap;
    use crate::{AsyncMap, Factory};
    #[tokio::test]
    async fn get_sync() {
        let map = VersionedMap::<String, String>::new();

        assert_eq!(None, map.get_if_present(&"foo".to_owned()));
    }

    fn hello_factory(key: &String) -> String {
        format!("Hello, {}!", key)
    }

    #[tokio::test]
    async fn get_sync2() {
        let map = VersionedMap::<String, String>::new();

        let key = "foo".to_owned();

        let future = map.get(
            &key,
            Box::new(hello_factory) as Box<dyn Factory<String, String>>,
        );

        assert_eq!(None, map.get_if_present(&key));
        let value = future.await;

        assert_eq!("Hello, foo!", value);
        assert_eq!("Hello, foo!", map.get_if_present(&key).unwrap());
    }
}
