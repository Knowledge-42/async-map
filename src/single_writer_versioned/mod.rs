//! The `single_writer_versioned` module implements a versioning mechanism for immutable data-structures
//! which allows many concurrent readers. All changes to the contained data structure - which, since they
//! are immtuable, means a creating a new instance - are delegated to a single task, and hence occur
//! sequentially. Each new update is appended to a linked list of versions, and an atomic integer is used to
//! indicate which is the latest version, so readers can retrieve the latest version when they read. This integer
//! acts in place of a lock on the linked list element, so that no actual locks are required and reads can always
//! proceed without waiting.
mod private {
    use std::cell::Cell;
    use std::ops::Deref;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::sync::Arc;

    pub trait Data: Send + Sync + std::fmt::Debug + 'static {}
    impl<T: Send + Sync + std::fmt::Debug + 'static> Data for T {}

    pub struct Version<T>
    where
        T: Data,
    {
        version: u32,
        data: T,
        next: Cell<Option<Arc<Version<T>>>>,
        latest_version: Arc<AtomicU32>,
    }

    impl<T: Data> std::fmt::Debug for Version<T> {
        fn fmt<'a>(&self, f: &mut std::fmt::Formatter<'a>) -> std::fmt::Result {
            f.write_fmt(format_args!("Version[{}]", self.version))
        }
    }

    impl<T> Version<T>
    where
        T: Data,
    {
        pub fn initial(data: T) -> (Arc<Version<T>>, Updater<T>) {
            let initial_version =
                Arc::new(Version::new_version(data, 0, Arc::new(AtomicU32::new(0))));
            let updater = Updater {
                version: initial_version.clone(),
            };
            (initial_version, updater)
        }

        pub fn as_ref(&self) -> &T {
            &self.data
        }

        pub fn latest<'a>(self: &'a Arc<Version<T>>) -> Option<&'a Arc<Version<T>>> {
            let latest_version = self.latest_version.load(Ordering::Acquire);

            if self.version == latest_version {
                None
            } else {
                // This is safe because above we ensured that
                // a) The requested version exists and is the latest one
                // b) It is not self, so self must be in the chain after self.
                Some(self.get_version(latest_version))
            }
        }

        /// This method is unsafe because it does not check whether there is a next; it
        /// should only be called in cases where that check has been performed
        fn next<'a>(self: &'a Arc<Version<T>>) -> &'a Arc<Version<T>> {
            unsafe { &*self.next.as_ptr() }.as_ref().unwrap()
        }

        /// This method is unsafed because it does not check whether the requested version exists,
        /// and is in the chain following self; it should only be called in cases where that check
        /// has been performed
        fn get_version<'a>(self: &'a Arc<Version<T>>, version: u32) -> &'a Arc<Version<T>> {
            if self.version == version {
                self
            } else {
                self.next().get_version(version)
            }
        }

        fn set_next(&self, data: T) -> Result<(Arc<Version<T>>, Updater<T>), T> {
            let latest_version = self.latest_version.load(Ordering::Acquire);

            // If this instance is not the latest version...
            if latest_version != self.version {
                // ...then next must already be set, so return data as
                return Err(data);
            }

            let new_version = latest_version + 1;

            let next = Arc::new(Version::new_version(
                data,
                new_version,
                self.latest_version.clone(),
            ));

            // First, set the next version on self
            self.next.replace(Some(next.clone()));

            // Now that next has been set we can update the latest version to reflect
            // the new reality
            self.latest_version.store(new_version, Ordering::Release);
            let updater = Updater {
                version: next.clone(),
            };
            Ok((next, updater))
        }

        fn new_version(data: T, version: u32, latest_version: Arc<AtomicU32>) -> Version<T> {
            let result = Version {
                version,
                data,
                next: Cell::new(None),
                latest_version,
            };
            result
        }
    }

    unsafe impl<T> Send for Version<T> where T: Data {}
    unsafe impl<T> Sync for Version<T> where T: Data {}

    impl<T> Deref for Version<T>
    where
        T: Data,
    {
        type Target = T;
        fn deref(&self) -> &T {
            &self.data
        }
    }

    pub struct Updater<T>
    where
        T: Data,
    {
        version: Arc<Version<T>>,
    }

    impl<T> Updater<T>
    where
        T: Data,
    {
        pub fn update(self, new_data: T) -> (Arc<Version<T>>, Updater<T>) {
            self.version.set_next(new_data).expect("Illegal State") // Is recovery possible?
        }
    }

    #[cfg(test)]
    mod test {
        #![allow(mutable_transmutes)]
        use super::Version;
        #[test]
        fn it_creates_sensible_initial() {
            let version = Version::initial("hello").0;
            assert_eq!("hello", version.data);
            assert_eq!(0, version.version);
        }

        #[test]
        fn it_accepts_a_next_version() {
            let (first, _) = Version::initial("hello");
            let (second, _) = first.set_next("goodbye").unwrap();

            assert_eq!("hello", first.data);
            assert_eq!(0, first.version);
            assert_eq!(second.version, first.next().as_ref().version);

            assert_eq!("goodbye", second.data);
            assert_eq!(1, second.version);
        }

        #[test]
        fn it_does_not_update_next_version() {
            let (first, _) = Version::initial("hello");
            let (_, _) = first.set_next("goodbye").unwrap();
            let result = first.set_next("au revoir");

            assert_eq!(true, result.is_err());
        }

        #[tokio::test]
        async fn it_can_be_used_across_tasks() {
            let version = Version::initial("hello").0;

            version.set_next("goodbye").unwrap();

            tokio::task::spawn(async move {
                assert_eq!("goodbye", version.next().data);
            })
            .await
            .unwrap();
        }

        #[test]
        fn latest_returns_none_on_latest() {
            let first = Version::initial("hello").0;

            assert_eq!(true, first.latest().is_none());

            let second = first.set_next("goodbye").unwrap().0;
            assert_eq!(true, second.latest().is_none());
        }

        #[test]
        fn latest_returns_latest() {
            let first = Version::initial("hello").0;

            let second = first.set_next("goodbye").unwrap().0;
            assert_eq!("goodbye", first.latest().unwrap().data);

            let third = second.set_next("servus").unwrap().0;
            assert_eq!("servus", first.latest().unwrap().data);
            assert_eq!("servus", second.latest().unwrap().data);
            assert_eq!(true, third.latest().is_none());
        }

        #[test]
        fn updater_updates() {
            let (first, updater) = Version::initial("hello");

            let (second, updater) = updater.update("goodbye");
            assert_eq!("goodbye", first.latest().unwrap().data);

            let third = updater.update("servus").0;
            assert_eq!("servus", first.latest().unwrap().data);
            assert_eq!("servus", second.latest().unwrap().data);
            assert_eq!(true, third.latest().is_none());
        }
    }
}

use self::private::{Data, Updater, Version};
use std::cell::RefCell;
use std::sync::Arc;
use tokio::sync::mpsc::{self, unbounded_channel, UnboundedReceiver, UnboundedSender};

pub trait DataUpdater<T>: (FnOnce(&T) -> Option<T>) + Send + 'static
where
    T: Data,
{
}

impl<T, S: (FnOnce(&T) -> Option<T>) + Send + 'static> DataUpdater<T> for S where T: Data {}

enum VersionedUpdaterAction<T>
where
    T: Data,
{
    Update(Box<dyn DataUpdater<T>>),
    Quit,
}

/// The core structure of this package, which provides synchronous read-access to the latest version and
/// aysnchronous writes, delegated to the update task.
///
/// The struct is not Sync, but it is Send; in order to share between tasks it should be cloned and Sent.
/// The contained data is not cloned; clones share the same backing linked list of versions and data.
///
/// Old versions are not actively purged, but will be dropped as long as there are no more instances holding
/// that version. This means that instances that are held for a long duration without being accessed will prevent
/// the old version from being purged. This situation should be avoided.
#[derive(Clone, Debug)]
pub struct Versioned<T>
where
    T: Data,
{
    current_holder: RefCell<Arc<Version<T>>>,
    update_sender: UnboundedSender<VersionedUpdaterAction<T>>,
}

/// Quits the updated task backing the data structure. Though not necessary (see notes) it allows
/// the the data structure to become invalidated for updates, which may speed up dropping of
/// references to it by acting as a signal to reference holders.
///
/// Some notes:
///
/// 1. Any previously dispatched updates will be
/// processed before the map task is quit
/// 1. As long as references to the Versioned itself, the data will not
/// be dropped. In other words, this does not free any memory.
/// 1. Conversely, when all references are dropped the memory and the task
/// will also be dropped. Thus, quitting is not necessary.
pub struct Quitter<T>
where
    T: Data,
{
    update_sender: UnboundedSender<VersionedUpdaterAction<T>>,
}

impl<T> Quitter<T>
where
    T: Data,
{
    pub fn quit(self) {
        if let Err(_) = self.update_sender.send(VersionedUpdaterAction::Quit) {
            // Probably already quit
        }
    }
}

impl<T> Versioned<T>
where
    T: Data,
{
    /// Creates the Versioned from the initial data, returning both the Versioned instance
    /// and a Quitter which can be used to stop the backing update task.
    pub fn from_initial(data: T) -> (Self, Quitter<T>) {
        let (initial_version, update_sender) = VersionedUpdater::start_from_initial(data);

        (
            Versioned {
                current_holder: RefCell::from(initial_version),
                update_sender: update_sender.clone(),
            },
            Quitter { update_sender },
        )
    }

    /// Passes a reference to the latest version of the contained data to the provided
    /// function and returns it result.
    ///
    /// This is the mechanism for read access to the data.
    pub fn with_latest<U, F: FnOnce(&T) -> U>(&self, action: F) -> U {
        self.ensure_latest();
        let the_ref = self.current_holder.borrow();
        action(&***the_ref)
    }

    fn ensure_latest(&self) {
        let current = self.current_holder.borrow();

        if let Some(new_version) = current.latest() {
            let new_version = new_version.clone();
            drop(current); // drop existing borrow.
            self.current_holder.replace(new_version);
        }
    }

    /// Allows the data to be upated by passing the latest version to the provided DataUpdater,
    /// and storing the result if one is provided. The update is delegated to the update task,
    /// which is also where the DataUpdater will be called.
    pub fn update(
        &self,
        update_fn: Box<dyn DataUpdater<T>>,
    ) -> Result<(), Box<dyn DataUpdater<T>>> {
        self.update_sender
            .send(VersionedUpdaterAction::Update(update_fn))
            .map_err(|action| match action {
                mpsc::error::SendError(VersionedUpdaterAction::Update(update_fn)) => update_fn,
                _ => panic!("Received illegal error"),
            })
    }
}

/// This is the backing task which receives the updates and performs them sequentially.
struct VersionedUpdater<T>
where
    T: Data,
{
    current: (Arc<Version<T>>, Updater<T>),
    update_receiver: UnboundedReceiver<VersionedUpdaterAction<T>>,
}

impl<T> VersionedUpdater<T>
where
    T: Data,
{
    fn start_from_initial(
        data: T,
    ) -> (Arc<Version<T>>, UnboundedSender<VersionedUpdaterAction<T>>) {
        let (initial_version, updater) = Version::initial(data);

        let (update_sender, update_receiver) = unbounded_channel();

        let current = (initial_version.clone(), updater);

        VersionedUpdater {
            current,
            update_receiver,
        }
        .run();

        (initial_version, update_sender)
    }

    fn run(mut self) {
        tokio::task::spawn(async move {
            while let Some(action) = self.update_receiver.recv().await {
                match action {
                    VersionedUpdaterAction::Update(update_fn) => {
                        if let Some(new_data) = update_fn(self.current.0.as_ref().as_ref()) {
                            self.current = self.current.1.update(new_data);
                        }
                    }
                    VersionedUpdaterAction::Quit => {
                        break;
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    #[tokio::test]
    async fn intial_holds_passed_data() {
        let versioned = Versioned::from_initial(String::from("Hello")).0;

        versioned.with_latest(|data| assert_eq!("Hello", data));
    }

    #[tokio::test]
    async fn updates_are_processed() {
        let versioned = Versioned::from_initial(String::from("Hello")).0;

        //updates affect versioned
        versioned
            .update(Box::new(|old| Some(old.clone() + ", World")))
            .map_err(|_| ())
            .expect("Should be ok");

        tokio::task::yield_now().await;

        versioned.with_latest(|data| assert_eq!("Hello, World", data));
    }

    #[tokio::test]
    async fn updates_are_shared() {
        let versioned = Versioned::from_initial(String::from("Hello")).0;
        let clone = versioned.clone();
        //updates affect versioned
        versioned
            .update(Box::new(|old| Some(old.clone() + ", World")))
            .map_err(|_| ())
            .expect("Should be ok");

        tokio::task::yield_now().await;

        versioned.with_latest(|data| assert_eq!("Hello, World", data));
        clone.with_latest(|data| assert_eq!("Hello, World", data));
    }

    #[tokio::test]
    async fn quitter_quits() {
        let tuple = Versioned::from_initial(String::from("Hello"));
        let versioned = tuple.0;
        let quitter = tuple.1;

        //updates affect versioned
        versioned
            .update(Box::new(|old| Some(old.clone() + ", World")))
            .map_err(|_| ())
            .expect("Should be ok");
        tokio::task::yield_now().await;

        quitter.quit();
        tokio::task::yield_now().await;

        let res = versioned.update(Box::new(|old| Some(old.clone() + "! And Moon!")));

        assert_eq!(true, res.is_err());

        tokio::task::yield_now().await;
        // The second update did not take.
        versioned.with_latest(|data| assert_eq!("Hello, World", data));
    }

    #[derive(Debug)]
    struct TestData {
        drop_counter: Arc<AtomicU32>,
    }

    impl Drop for TestData {
        fn drop(&mut self) {
            self.drop_counter.fetch_add(1, Ordering::Release);
        }
    }

    #[tokio::test]
    async fn old_versions_are_purged() {
        let counter = Arc::<AtomicU32>::default();
        let drop_counter = counter.clone();

        let versioned: Versioned<Arc<TestData>> = Versioned::from_initial(Arc::new(TestData {
            drop_counter: drop_counter,
        }))
        .0;
        let clone = versioned.clone();

        assert_eq!(0, counter.load(Ordering::Acquire));

        let drop_counter = counter.clone();

        //updates affect versioned
        versioned
            .update(Box::new(|_| {
                Some(Arc::new(TestData {
                    drop_counter: drop_counter,
                }))
            }))
            .map_err(|_| ())
            .expect("Should be ok");

        tokio::task::yield_now().await;
        // update Versioned to latest
        versioned.with_latest(Box::new(|_: &Arc<TestData>| ()));
        tokio::task::yield_now().await;

        // Nothing dropped because clone still has initial version
        assert_eq!(0, counter.load(Ordering::Acquire));

        // update cloned to latest
        clone.with_latest(Box::new(|_: &Arc<TestData>| ()));
        tokio::task::yield_now().await;
        // First verion dropped, everyone has latest
        assert_eq!(1, counter.load(Ordering::Acquire));

        drop(versioned);
        drop(clone);

        tokio::task::yield_now().await;

        // all Versioned instances have been dropped, so the update task should have ended
        // and the latest version dropped too
        assert_eq!(2, counter.load(Ordering::Acquire));
    }

    fn any_test<T: std::any::Any + Send + 'static>(func: Box<dyn FnOnce() -> Box<T>>) -> Box<T> {
        func()
    }

    #[tokio::test]
    async fn test_any() {
        let foo = any_test(Box::new(|| Box::new(String::from("Hello"))));

        assert_eq!("Hello", *foo);

        let bar = any_test(Box::new(|| {
            Box::new(im::HashMap::new().update("key", "secret"))
        }));

        assert_eq!("secret", *bar.get("key").unwrap());
    }
}
