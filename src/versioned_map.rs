use std::cell::RefCell;

use std::future::{ready, Future};

use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll, Waker};

use crate::{AsyncMap, KeyTrait, ValueTrait};

use futures::FutureExt;

use im::HashMap;

use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot::{self, error::TryRecvError, Receiver, Sender};

enum MapAction<K: KeyTrait, V: ValueTrait> {
    GetOrCreate(
        K,
        Box<dyn Fn(&K) -> V + Send>,
        Sender<(V, MapHolder<K, V>)>,
        Waker,
    ),
}

struct MapReturnFuture<'a, K: KeyTrait, V: ValueTrait> {
    update_sender: UnboundedSender<MapAction<K, V>>,
    key: &'a K,
    factory: Option<Box<dyn Fn(&K) -> V + Send>>,
    result_sender: Option<Sender<(V, MapHolder<K, V>)>>,
}

impl<'a, K: KeyTrait, V: ValueTrait> Future for MapReturnFuture<'a, K, V> {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut mutable = self;

        if mutable.result_sender.is_none() {
            Poll::Ready(())
        } else {
            let result_sender = mutable.result_sender.take().unwrap();
            match mutable.factory.take() {
                None => {
                    todo!()
                }
                Some(factory) => {
                    match mutable.update_sender.send(MapAction::GetOrCreate(
                        mutable.key.clone(),
                        factory,
                        result_sender,
                        cx.waker().clone(),
                    )) {
                        Ok(_) => Poll::Pending,
                        Err(_) => Poll::Pending,
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct MapHolder<K: KeyTrait, V: ValueTrait> {
    version: u64,
    map: HashMap<K, V>,
}

impl<K: KeyTrait, V: ValueTrait> MapHolder<K, V> {
    pub fn is_version(&self, version: u64) -> bool {
        self.version == version
    }
}

pub struct VersionedMap<K: KeyTrait, V: ValueTrait> {
    latest_version: Arc<AtomicU64>,
    map_holder: RefCell<MapHolder<K, V>>,
    update_sender: UnboundedSender<MapAction<K, V>>,
    result_receiver: RefCell<Option<Receiver<MapHolder<K, V>>>>,
}

impl<K: KeyTrait, V: ValueTrait> AsyncMap for VersionedMap<K, V> {
    type Key = K;
    type Value = V;

    /// Synchronously returns the value associated with the provided key, if present; otherwise None
    fn get_if_present(&self, key: &Self::Key) -> Option<Self::Value> {
        if self.result_receiver.borrow().is_some() {
            let mut receiver = self.result_receiver.take().unwrap();
            match receiver.try_recv() {
                Err(TryRecvError::Empty) => {
                    // Not ready yet - put it back
                    self.result_receiver.replace(Some(receiver));
                }
                Err(TryRecvError::Closed) => {
                    println!("get_if_present: closed");
                    std::process::exit(-1);
                }
                Ok(holder) => {
                    self.map_holder.replace(holder);
                }
            }
        }

        let map_holder = self.map_holder.borrow();

        if map_holder.is_version(self.latest_version.load(Ordering::Acquire)) {
            map_holder.map.get(key).map(V::clone)
        } else {
            None // todo: Should probably be future
        }
    }

    fn get<'a>(
        &self,
        key: &'a Self::Key,
        factory: Box<dyn Fn(&Self::Key) -> Self::Value + Send + 'static>,
    ) -> Pin<Box<dyn Future<Output = Self::Value> + Send + 'a>> {
        match self.get_if_present(key) {
            Some(x) => Box::pin(ready(x)),
            None => {
                let (tx, mut rx) = oneshot::channel();
                let (tx2, rx2) = oneshot::channel();
                self.result_receiver.replace(Some(rx2));

                Box::pin(
                    (MapReturnFuture {
                        key,
                        factory: Some(factory),
                        update_sender: self.update_sender.clone(),
                        result_sender: Some(tx),
                    })
                    .then(move |_| match rx.try_recv() {
                        Err(_) => {
                            std::process::exit(-1);
                        }
                        Ok((value, map_holder)) => {
                            if let Err(_) = tx2.send(map_holder) {
                                todo!()
                            }
                            ready(value)
                        }
                    }),
                )
            }
        }
    }
}

impl<K: KeyTrait, V: ValueTrait> Clone for VersionedMap<K, V> {
    fn clone(&self) -> Self {
        VersionedMap {
            latest_version: self.latest_version.clone(),
            map_holder: self.map_holder.clone(),
            update_sender: self.update_sender.clone(),
            result_receiver: RefCell::default(), // The clone will start the process of listening for updates independently
        }
    }
}

impl<K: KeyTrait, V: ValueTrait> VersionedMap<K, V> {
    pub fn new() -> Self {
        let (update_sender, mut update_receiver) = mpsc::unbounded_channel();

        let initial_version = 0;
        let latest_version = Arc::new(AtomicU64::new(initial_version));
        let map = HashMap::default();

        let map_holder = MapHolder {
            version: initial_version,
            map: map.clone(),
        };

        let non_locking_map: VersionedMap<K, V> = VersionedMap {
            latest_version: latest_version.clone(),
            map_holder: RefCell::new(map_holder),
            update_sender,
            result_receiver: RefCell::default(),
        };

        Some(tokio::task::spawn(async move {
            let mut current_map = map;
            while let Some(action) = update_receiver.recv().await {
                match action {
                    MapAction::GetOrCreate(key, factory, result_sender, waker) => {
                        if let Some(new_map) = VersionedMap::create_if_necessary(
                            &latest_version,
                            &current_map,
                            key,
                            factory,
                            result_sender,
                        ) {
                            current_map = new_map;
                        }

                        waker.wake();
                    }
                }
            }
        }));

        non_locking_map
    }

    fn create_if_necessary(
        latest_version: &Arc<AtomicU64>,
        map: &HashMap<K, V>,
        key: K,
        factory: Box<dyn Fn(&K) -> V + Send>,
        result_sender: Sender<(V, MapHolder<K, V>)>,
    ) -> Option<HashMap<K, V>> {
        match map.get(&key) {
            Some(v) => {
                // nothing to do; probably multiple creates were queued up for the same key
                if let Err(_) = result_sender.send((
                    v.clone(),
                    MapHolder {
                        version: latest_version.load(Ordering::Acquire),
                        map: map.clone(),
                    },
                )) {
                    todo!()
                }
                None
            }
            None => {
                let value = factory(&key);

                // println!("Length: {}", map.len());
                let updated = map.update(key, value.clone());

                // 1 not added yet!
                let prior_version = latest_version.fetch_add(1, Ordering::AcqRel);

                if let Err(_) = result_sender.send((
                    value,
                    MapHolder {
                        version: prior_version + 1,
                        map: updated.clone(),
                    },
                )) {
                    todo!()
                }
                Some(updated)
            }
        }
    }
}

#[cfg(test)]
mod test {

    use super::VersionedMap;
    use crate::AsyncMap;
    #[tokio::test]
    async fn get_sync() {
        let map = VersionedMap::<String, String>::new();

        assert_eq!(None, map.get_if_present(&"foo".to_owned()));
    }

    #[tokio::test]
    async fn get_sync2() {
        let map = VersionedMap::<String, String>::new();

        let key = "foo".to_owned();

        let future = map.get(&key, Box::new(|key| format!("Hello, {}!", key)));

        assert_eq!(None, map.get_if_present(&key));
        let value = future.await;

        assert_eq!("Hello, foo!", value);
        assert_eq!("Hello, foo!", map.get_if_present(&key).unwrap());
    }
}
