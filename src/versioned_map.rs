use std::cell::{Ref, RefCell};

use std::future::{ready, Future};

use std::boxed::Box;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll, Waker};

use crate::{AsyncMap, FactoryBorrow, KeyTrait, ValueTrait};

use futures::FutureExt;

use im::HashMap;

use tokio::sync::mpsc::{self, UnboundedSender};
use tokio::sync::oneshot;

enum MapAction<K: KeyTrait, V: ValueTrait> {
    GetOrCreate(
        K,
        Box<dyn FactoryBorrow<K, V>>,
        oneshot::Sender<(V, MapHolder<K, V>)>,
        Waker,
    ),
}

struct MapReturnFuture<K: KeyTrait, V: ValueTrait, B>
where
    B: FactoryBorrow<K, V> + Unpin,
{
    update_sender: UnboundedSender<MapAction<K, V>>,
    key: K,
    factory: Option<B>,
    result_sender: Option<oneshot::Sender<(V, MapHolder<K, V>)>>,
}

impl<'a, K: KeyTrait, V: ValueTrait, B> Future for MapReturnFuture<K, V, B>
where
    B: FactoryBorrow<K, V> + Unpin,
{
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
                        Box::new(factory),
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

pub struct VersionedMap<K: KeyTrait, V: ValueTrait> {
    latest_version: Arc<AtomicU64>,
    map_holder: RefCell<MapHolder<K, V>>,
    update_sender: UnboundedSender<MapAction<K, V>>,
    update_receiver: UpdateReceiver<K, V>,
    latest_map_holder: Arc<RwLock<MapHolder<K, V>>>,
}

struct UpdateReceiver<K: KeyTrait, V: ValueTrait> {
    receiver: RefCell<Option<oneshot::Receiver<MapHolder<K, V>>>>,
}

impl<K: KeyTrait, V: ValueTrait> Default for UpdateReceiver<K, V> {
    fn default() -> Self {
        UpdateReceiver {
            receiver: RefCell::new(None),
        }
    }
}

impl<K: KeyTrait, V: ValueTrait> UpdateReceiver<K, V> {
    pub fn updater(&self) -> MapUpdater<K, V> {
        let (sender, receiver) = oneshot::channel();
        // Note that any prior receiver will be lost. Since updates are
        // linear, that is not an issue
        self.receiver.replace(Some(receiver));
        MapUpdater { sender }
    }

    pub fn get_update(&self) -> Option<MapHolder<K, V>> {
        self.receiver.take().and_then(|mut receiver| {
            match receiver.try_recv() {
                Err(oneshot::error::TryRecvError::Empty) => {
                    // Not ready yet - put it back
                    self.receiver.replace(Some(receiver));
                    None
                }
                Err(oneshot::error::TryRecvError::Closed) => {
                    println!("get_if_present: closed");
                    std::process::exit(-1);
                }
                Ok(holder) => Some(holder),
            }
        })
    }
}

struct MapUpdater<K: KeyTrait, V: ValueTrait> {
    sender: oneshot::Sender<MapHolder<K, V>>,
}

impl<K: KeyTrait, V: ValueTrait> MapUpdater<K, V> {
    pub fn apply(self, new_map: MapHolder<K, V>) {
        if let Err(_) = self.sender.send(new_map) {
            // probably the map was alread dropped; ignore
        }
    }
}

impl<K: KeyTrait, V: ValueTrait> AsyncMap for VersionedMap<K, V> {
    type Key = K;
    type Value = V;

    /// Synchronously returns the value associated with the provided key, if present; otherwise None
    fn get_if_present(&self, key: &Self::Key) -> Option<Self::Value> {
        self.latest_map().map.get(key).map(V::clone)
    }

    fn get<'a, 'b, B: FactoryBorrow<K, V>>(
        &'a self,
        key: &'a Self::Key,
        factory: B,
    ) -> Pin<Box<dyn Future<Output = Self::Value> + Send + 'b>> {
        match self.get_if_present(key) {
            Some(x) => Box::pin(ready(x)),
            None => self.send_update(key.clone(), factory),
        }
    }
}

impl<K: KeyTrait, V: ValueTrait> Clone for VersionedMap<K, V> {
    fn clone(&self) -> Self {
        VersionedMap {
            latest_version: self.latest_version.clone(),
            map_holder: self.map_holder.clone(),
            update_sender: self.update_sender.clone(),
            update_receiver: UpdateReceiver::default(), // The clone will start the process of listening for updates independently
            latest_map_holder: self.latest_map_holder.clone(),
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

        let current_map_holder = Arc::new(RwLock::new(MapHolder {
            version: initial_version,
            map: map,
        }));

        let non_locking_map: VersionedMap<K, V> = VersionedMap {
            latest_version: latest_version.clone(),
            map_holder: RefCell::new(map_holder),
            update_sender,
            update_receiver: UpdateReceiver::default(),
            latest_map_holder: current_map_holder.clone(),
        };

        Some(tokio::task::spawn(async move {
            let lockable_map_holder = current_map_holder;
            while let Some(action) = update_receiver.recv().await {
                match action {
                    MapAction::GetOrCreate(key, factory, result_sender, waker) => {
                        let read_lock = lockable_map_holder.read();

                        let updated = match read_lock {
                            Err(_) => todo!(),
                            Ok(map_holder) => VersionedMap::create_if_necessary(
                                &latest_version,
                                &map_holder.map,
                                key,
                                factory,
                                result_sender,
                            ),
                        }; // Read lock dropped here.

                        if let Some((new_map, new_version)) = updated {
                            let write_lock = lockable_map_holder.write();

                            match write_lock {
                                Err(_) => todo!(),
                                Ok(mut map_holder) => {
                                    map_holder.version = new_version;
                                    map_holder.map = new_map;
                                }
                            }
                        }

                        waker.wake();
                    }
                }
            }
        }));

        non_locking_map
    }

    fn send_update<'a, 'b, B: FactoryBorrow<K, V>>(
        &self,
        key: K,
        factory: B,
    ) -> Pin<Box<dyn Future<Output = V> + Send + 'b>> {
        let (tx, mut rx) = oneshot::channel();
        let map_updater = self.get_updater();

        self.create_return_future(key, factory, tx)
            .then(move |_| match rx.try_recv() {
                Err(_) => {
                    std::process::exit(-1);
                }
                Ok((value, map_holder)) => {
                    map_updater.apply(map_holder);
                    ready(value)
                }
            })
            .boxed()
    }

    fn create_return_future<B: FactoryBorrow<K, V>>(
        &self,
        key: K,
        factory: B,
        sender: oneshot::Sender<(V, MapHolder<K, V>)>,
    ) -> MapReturnFuture<K, V, B> {
        MapReturnFuture {
            key: key,
            factory: Some(factory),
            update_sender: self.update_sender.clone(),
            result_sender: Some(sender),
        }
    }

    fn get_updater(&self) -> MapUpdater<K, V> {
        self.update_receiver.updater()
    }

    fn latest_map(&self) -> Ref<MapHolder<K, V>> {
        let latest_version = self.latest_version.load(Ordering::Acquire);

        // Get any update received from a write op, filtering on version
        let received_update = self
            .get_received_update()
            .filter(|holder| holder.version == latest_version);
        if let Some(new_map_holder) = received_update {
            self.map_holder.replace(new_map_holder);
        } else {
            let mut current = self.map_holder.borrow_mut();

            if current.version != latest_version {
                let latest = self.get_latest();

                current.map = latest.map;
                current.version = latest.version;
            }
        }

        self.map_holder.borrow()
    }

    fn get_received_update(&self) -> Option<MapHolder<K, V>> {
        self.update_receiver.get_update()
    }

    fn get_latest(&self) -> MapHolder<K, V> {
        let lock_result = self.latest_map_holder.read();

        match lock_result {
            Err(_) => todo!(),
            Ok(guard) => {
                let latest_holder = guard.clone();
                latest_holder
            }
        }
    }

    fn create_if_necessary(
        latest_version: &Arc<AtomicU64>,
        map: &HashMap<K, V>,
        key: K,
        factory: Box<dyn FactoryBorrow<K, V>>,
        result_sender: oneshot::Sender<(V, MapHolder<K, V>)>,
    ) -> Option<(HashMap<K, V>, u64)> {
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
                let value = (*factory).borrow()(&key);

                // println!("Length: {}", map.len());
                let updated = map.update(key, value.clone());

                // fetch_add returns the prior value!
                let new_version = latest_version.fetch_add(1, Ordering::AcqRel) + 1;

                if let Err(_) = result_sender.send((
                    value,
                    MapHolder {
                        version: new_version,
                        map: updated.clone(),
                    },
                )) {
                    todo!()
                }
                Some((updated, new_version))
            }
        }
    }
}

#[cfg(test)]
mod test {

    use super::VersionedMap;
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
