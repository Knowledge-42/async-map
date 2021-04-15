use std::cmp::Eq;
use std::collections::VecDeque;
use std::future::{ready, Future};
use std::hash::Hash;
use std::pin::Pin;
use std::sync::atomic::{AtomicPtr, Ordering};
use std::sync::Arc;

use std::task::{Context, Poll, Waker};

use im::HashMap;

use tokio::sync::mpsc::{self, UnboundedSender};

enum MapAction<K, V>
where
    K: Clone + Hash + Eq + Sync + Send + Unpin,
    V: Clone + Sync + Send + Unpin,
{
    GetOrCreate(K, Box<dyn Fn(&K) -> V + Send>, Waker),
    Quit,
}

struct MapReturnFuture<'a, K, V>
where
    K: Clone + Hash + Eq + Sync + Send + Unpin,
    V: Clone + Sync + Send + Unpin,
{
    update_sender: UnboundedSender<MapAction<K, V>>,
    map_ptr: Arc<AtomicPtr<Arc<HashMap<K, V>>>>,
    key: &'a K,
    factory: Option<Box<dyn Fn(&K) -> V + Send>>,
}

impl<'a, K, V> Future for MapReturnFuture<'a, K, V>
where
    K: 'static + Clone + Hash + Eq + Sync + Send + Unpin + std::fmt::Debug,
    V: Clone + Sync + Send + Unpin + std::fmt::Debug + 'static,
{
    type Output = V;
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut mutable = self;
        match NonLockingMap::get_sync(&mutable.map_ptr, &mutable.key) {
            Some(value) => Poll::Ready(value),
            None => match mutable.factory.take() {
                None => {
                    println!("Option is None for key {:?}!", *mutable.key);
                    panic!()
                }
                Some(factory) => {
                    match mutable.update_sender.send(MapAction::GetOrCreate(
                        mutable.key.clone(),
                        factory,
                        cx.waker().clone(),
                    )) {
                        Ok(_) => Poll::Pending,
                        Err(_) => Poll::Pending,
                    }
                }
            },
        }
    }
}

struct Swappable<T> {
    instance: T,
}

impl<T> Swappable<T> {
    fn new(instance: T) -> Self {
        Swappable { instance }
    }

    fn swap(&mut self, new_instance: T) -> T {
        std::mem::replace(&mut self.instance, new_instance)
    }
}

#[derive(Clone)]
pub struct NonLockingMap<K, V>
where
    K: Clone + Hash + Eq + Sync + Send + Unpin + 'static,
    V: Clone + Sync + Send + Unpin + 'static,
{
    update_sender: UnboundedSender<MapAction<K, V>>,
    map_ptr: Arc<AtomicPtr<Arc<HashMap<K, V>>>>,
    //update_task_handle: Option<JoinHandle<()>>
}

impl<K, V> NonLockingMap<K, V>
where
    K: 'static + Clone + Hash + Eq + Sync + Send + Unpin + std::fmt::Debug,
    V: 'static + Clone + Sync + Send + Unpin + std::fmt::Debug,
{
    fn get_sync<'a>(map_ptr: &Arc<AtomicPtr<Arc<HashMap<K, V>>>>, key: &'a K) -> Option<V> {
        unsafe {
            // println!("Cloning Arc {:?} for {:?}", map_ptr, key);
            let arc = map_ptr.load(Ordering::Acquire);
            // println!(
            //     "Incremented count: {}, {}",
            //     Arc::strong_count(&arc),
            //     arc.len()
            // );
            // println!("Got map");

            let val = (*arc).get(key).map(|val| val.clone());
            // println!("Got val: {:?}", val);
            val
        }
    }

    pub fn new() -> Self {
        let (update_sender, mut update_receiver) = mpsc::unbounded_channel();

        let mut map = Arc::pin(Arc::new(HashMap::default()));

        let map_ptr: Arc<AtomicPtr<Arc<HashMap<K, V>>>>;

        unsafe {
            map_ptr = Arc::new(AtomicPtr::new(&*map as *const _ as *mut _));
            // println!(
            //     "Strong: {}, {}, {}",
            //     Arc::strong_count(&*map_ptr.load(Ordering::Acquire)),
            //     map.len(),
            //     (*map_ptr.load(Ordering::Acquire)).len(),
            // );
        }

        let non_locking_map: NonLockingMap<K, V> = NonLockingMap {
            update_sender,
            map_ptr,
            //update_task_handle: None
        };

        let mut cloned = non_locking_map.clone();
        let mut maps = VecDeque::new();
        Some(tokio::task::spawn(async move {
            while let Some(action) = update_receiver.recv().await {
                match action {
                    MapAction::Quit => break,
                    MapAction::GetOrCreate(key, factory, waker) => {
                        // let logkey = key.clone();
                        // println!("Creating key: {:?}", logkey);
                        if let Some(new_map) = cloned.create_if_necessary(&map, key, factory) {
                            maps.push_back(map);
                            if maps.len() > 10 {
                                maps.pop_front();
                            }
                            map = new_map;
                        }
                        // unsafe {
                        //     let foo = (*cloned.map_ptr.load(Ordering::Acquire)).clone();
                        //     // println!(
                        //     //     "Checking...{},{:?}",
                        //     //     Arc::strong_count(&foo),
                        //     //     foo.get(&logkey)
                        //     // );
                        // }
                        // if let Some(x) = cloned.get_sync_instance(&logkey) {
                        //     println!("Value in ptr map: {:?}", x);
                        // }
                        waker.wake(); // Let the calling thread know that the map now has a value for this key
                                      //println!("Created key: {:?}", logkey);
                    }
                }
            }
            println!("Stopping update task, {}", map.len());
        }));

        non_locking_map
    }

    fn create_if_necessary(
        &mut self,
        map: &Pin<Arc<Arc<HashMap<K, V>>>>,
        key: K,
        factory: Box<dyn Fn(&K) -> V + Send>,
    ) -> Option<Pin<Arc<Arc<HashMap<K, V>>>>> {
        match self.get_sync_instance(&key) {
            Some(_) => {
                // nothing to do; probably multiple creates were queued up for the same key
                None
            }
            None => {
                let value = factory(&key);

                // println!("Length: {}", map.len());
                let updated = Arc::new(map.update(key.clone(), value));
                // println!("Length: {}", updated.len());

                let new_map = Arc::pin(updated);

                // unsafe {
                self.map_ptr
                    .store(&*new_map as *const _ as *mut _, Ordering::Release);
                // println!(
                //     "Strong: {}, {}, {}",
                //     Arc::strong_count(&*self.map_ptr.load(Ordering::Acquire)),
                //     new_map.len(),
                //     (*self.map_ptr.load(Ordering::Acquire)).len()
                // );
                // }

                // println!("Storing Ptr {:?}", self.map_ptr.load(Ordering::Acquire));
                self.map_ptr
                    .store(self.map_ptr.load(Ordering::Acquire), Ordering::Release);

                // println!("Stored Ptr");
                // if let None = self.get_sync_instance(&key) {
                //     panic!("No value in ptr map!");
                // }
                Some(new_map)
            }
        }
    }

    fn get_sync_instance(&self, key: &K) -> Option<V> {
        NonLockingMap::get_sync(&self.map_ptr, key)
    }

    /// Synchronously returns the value associated with the provided key, if present; otherwise None
    pub fn get_if_present(&self, key: &K) -> Option<V> {
        self.get_sync_instance(key)
    }

    pub fn get<'a>(
        &self,
        key: &'a K,
        factory: Box<dyn Fn(&K) -> V + Send + 'static>,
    ) -> impl Future<Output = V> + 'a {
        MapReturnFuture {
            key,
            factory: Some(factory),
            map_ptr: self.map_ptr.clone(),
            update_sender: self.update_sender.clone(),
        }
    }
}

#[cfg(test)]
mod test {

    use super::NonLockingMap;
    use im::HashMap;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicPtr, Ordering};
    use std::sync::Arc;
    // #[tokio::test]
    // async fn get_sync() {
    //     let map = NonLockingMap::<String, String>::new();

    //     assert_eq!(None, map.get_if_present(&"foo".to_owned()));
    // }

    // #[tokio::test]
    // async fn get_sync2() {
    //     let map = NonLockingMap::<String, String>::new();

    //     let key = "foo".to_owned();

    //     let future = map.get(&key, Box::new(|key| format!("Hello, {}!", key)));

    //     assert_eq!(None, map.get_if_present(&key));
    //     let value = future.await;

    //     assert_eq!("Hello, foo!", value);
    //     assert_eq!("Hello, foo!", map.get_if_present(&key).unwrap());
    // }

    fn create_clone() -> (
        Pin<Arc<Arc<HashMap<String, String>>>>,
        Arc<AtomicPtr<Arc<HashMap<String, String>>>>,
    ) {
        let mut map = Arc::pin(Arc::new(
            HashMap::<String, String>::default().update("hello".to_owned(), "darling".to_owned()),
        ));
        let new_ptr: *mut Arc<HashMap<String, String>>;

        new_ptr = &*map as *const _ as *mut _;

        unsafe {
            assert_eq!(1, Arc::strong_count(&*new_ptr));
        }

        let clone = (*map).clone();

        unsafe {
            assert_eq!(2, Arc::strong_count(&*new_ptr));
        }

        let clone2;
        unsafe {
            clone2 = (*new_ptr).clone();
        }

        assert_eq!(3, Arc::strong_count(&*map));

        assert_eq!("darling", clone2.get("hello").unwrap());

        (map, Arc::new(AtomicPtr::new(new_ptr)))
    }
    #[test]
    fn test_ptr() {
        let (map, ptr) = create_clone();

        let clone2;
        unsafe {
            clone2 = (*ptr.load(Ordering::Acquire)).clone();
        }

        assert_eq!(2, Arc::strong_count(&*map));

        assert_eq!("darling", clone2.get("hello").unwrap());
    }
}
