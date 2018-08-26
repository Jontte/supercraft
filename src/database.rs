use byteorder::{BigEndian, ByteOrder, WriteBytesExt};
use lmdb;
use rmp_serde;

use serde;
use snap;
use std::borrow::Borrow;
use std::io::{Cursor, Write};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Instant;
use uuid::Uuid;
struct Timer {
    instant: Instant,
}
impl Timer {
    pub fn new() -> Timer {
        Timer {
            instant: Instant::now(),
        }
    }
    pub fn elapsed(&self) -> f64 {
        let duration = self.instant.elapsed();
        duration.as_secs() as f64 + duration.subsec_nanos() as f64 * 1e-9
    }
}

#[derive(Serialize, Deserialize, Clone)]
pub struct DBUnitTest {
    pub uuid: Uuid,
    pub data: Vec<u8>,
}

#[derive(Serialize, Deserialize)]
pub struct DBHeader {
    references: Vec<Uuid>,
}

#[derive(Clone)]
pub struct Environment {
    env: lmdb::Environment,
}

pub struct Database {
    env: lmdb::Environment,
    db_handle: lmdb::DbHandle,
}

impl Database {
    pub fn create_env(path: &Path) -> Environment {
        // Multiple handles to the database can be opened simultaneously as long as they use same
        // environment returned by this function

        Environment {
            env: lmdb::EnvBuilder::new()
                .map_size(4 * 1024 * 1024 * 1024u64) // max database size
                //.flags(lmdb::core::EnvCreateNoMetaSync)
                .open(path, 0o755)
                .expect("unable to create lmdb environment"),
        }
    }
    pub fn new(env: Environment) -> Database {
        let db_handle = env
            .env
            .get_default_db(lmdb::DbFlags::empty())
            .expect("unable to create lmdb database");

        Database {
            env: env.env,
            db_handle: db_handle,
        }
    }

    pub fn put<K: lmdb::ToMdbValue, V: lmdb::ToMdbValue>(
        &mut self,
        key: &K,
        value: &V,
    ) -> Result<(), ()> {
        let txn = self
            .env
            .new_transaction()
            .expect("unable to start database transaction");
        let result = {
            let db = txn.bind(&self.db_handle);
            match db.set(key, value) {
                Ok(_) => Ok(()),
                Err(x) => {
                    println!("Database::put failed: {}", x);
                    Err(())
                }
            }
        };
        txn.commit().unwrap();
        result
    }

    pub fn put_many<K: lmdb::ToMdbValue, V: lmdb::ToMdbValue>(
        &mut self,
        items: &[(K, V)],
    ) -> Result<(), ()> {
        let txn = self
            .env
            .new_transaction()
            .expect("unable to start database transaction");
        {
            let db = txn.bind(&self.db_handle);
            for (key, value) in items.iter() {
                match db.set(key, value) {
                    Ok(_) => {}
                    Err(x) => {
                        println!("Database::put failed: {}", x);
                        return Err(());
                    }
                }
            }
        };
        txn.commit().unwrap();
        Ok(())
    }

    pub fn get<K: lmdb::ToMdbValue>(&mut self, key: &K) -> Result<Option<Vec<u8>>, ()> {
        let txn = match self.env.new_transaction() {
            Ok(x) => x,
            Err(x) => {
                println!("unable to start database transaction: {:?}", x);
                return Err(());
            }
        };
        let ret = {
            let db = txn.bind(&self.db_handle);
            match db.get(key) {
                Ok(x) => Ok(Some(x)),
                Err(_) => {
                    Ok(None) // Key not found
                }
            }
        };
        txn.commit().unwrap();
        ret
    }

    pub fn get_many<K: lmdb::ToMdbValue>(
        &mut self,
        keys: &[K],
    ) -> Result<Vec<Option<Vec<u8>>>, ()> {
        let txn = match self.env.get_reader() {
            Ok(x) => x,
            Err(x) => {
                println!("unable to start database transaction: {:?}", x);
                return Err(());
            }
        };
        let ret = {
            let db = txn.bind(&self.db_handle);

            keys.into_iter()
                .map(|key| {
                    match db.get(key) {
                        Ok(x) => Ok(Some(x)),
                        Err(_) => {
                            Ok(None) // Key not found
                        }
                    }
                })
                .collect()
        };
        ret
    }

    pub fn find<K: lmdb::ToMdbValue>(
        &mut self,
        begin: &K,
        end: &K,
    ) -> Result<Vec<(Vec<u8>, Vec<u8>)>, ()> {
        let txn = self
            .env
            .new_transaction()
            .expect("unable to start database transaction");
        let mut result = Vec::new();
        {
            let db = txn.bind(&self.db_handle);
            let iter = match db.keyrange_from_to(begin, end) {
                Ok(x) => x,
                Err(x) => {
                    println!("Database::put failed: {}", x);
                    return Err(());
                }
            };
            for keyval in iter {
                result.push((keyval.get_key(), keyval.get_value()));
            }
        }
        txn.commit().unwrap();
        Ok(result)
    }
    pub fn find_keys<K: lmdb::ToMdbValue>(
        &mut self,
        begin: &K,
        end: &K,
    ) -> Result<Vec<Vec<u8>>, ()> {
        let txn = self
            .env
            .new_transaction()
            .expect("unable to start database transaction");
        let mut result = Vec::new();
        {
            let db = txn.bind(&self.db_handle);
            let iter = match db.keyrange_from_to(begin, end) {
                Ok(x) => x,
                Err(x) => {
                    println!("Database::put failed: {}", x);
                    return Err(());
                }
            };
            for keyval in iter {
                result.push(keyval.get_key());
            }
        }
        txn.commit().unwrap();
        Ok(result)
    }
}

type ToWorkerMessage<Ctx> =
    Box<FnMut(&mpsc::Sender<FromWorkerMessage<Ctx>>, &mut Database) -> Result<(), ()> + Send>;

enum FromWorkerMessage<Ctx> {
    Callback(Box<FnMut(&mut Ctx) + Send>),
    Flush,
}

pub struct AsyncDatabase<Ctx> {
    to_worker_tx: mpsc::Sender<ToWorkerMessage<Ctx>>,
    from_worker_rx: mpsc::Receiver<FromWorkerMessage<Ctx>>,
}

#[allow(dead_code)]
impl<Ctx> AsyncDatabase<Ctx>
where
    Ctx: 'static,
{
    pub fn new(env: Environment) -> AsyncDatabase<Ctx> {
        let (to_worker_tx, to_worker_rx) = mpsc::channel();
        let (from_worker_tx, from_worker_rx) = mpsc::channel();

        let mut db = Database::new(env);

        thread::spawn(move || loop {
            match to_worker_rx.recv() {
                Ok(callback) => {
                    let mut callback: ToWorkerMessage<Ctx> = callback;

                    //let t = Timer::new();
                    if let Err(_) = callback(&from_worker_tx, &mut db) {
                        println!("DB worker killed");
                        break;
                    }
                    //println!("worker callback took {} s", t.elapsed());
                }
                Err(_) => {
                    break;
                }
            };
        });

        AsyncDatabase {
            to_worker_tx: to_worker_tx,
            from_worker_rx: from_worker_rx,
        }
    }

    pub fn put<K, V>(&mut self, key: K, value: V)
    where
        K: Into<Vec<u8>>,
        V: Into<Vec<u8>>,
    {
        let key: Vec<u8> = key.into();
        let value: Vec<u8> = value.into();

        self.to_worker_tx
            .send(Box::new(move |_, db| {
                db.put(&key.as_slice(), &value.as_slice()).unwrap();
                Ok(())
            }))
            .unwrap();
    }
    pub fn put_many(&mut self, items: Vec<(Vec<u8>, Vec<u8>)>) {
        self.to_worker_tx
            .send(Box::new(move |_, db| {
                db.put_many(&items).unwrap();
                Ok(())
            }))
            .unwrap();
    }
    pub fn get_ctx<K, F>(&mut self, key: K, callback: F)
    where
        K: Into<Vec<u8>> + Clone,
        F: FnMut(&mut Ctx, Option<Vec<u8>>) + Send + 'static,
    {
        let key: Vec<u8> = key.into();
        let mut callback: Option<Box<F>> = Some(Box::new(callback));

        self.to_worker_tx
            .send(Box::new(move |tx, db| {
                let mut callback = Some(callback.take().unwrap());
                let mut result = Some(db.get(&key.as_slice()).map_err(|_| ())?);
                tx.send(FromWorkerMessage::Callback(Box::new(move |ctx| {
                    callback.take().unwrap()(ctx, result.take().unwrap());
                }))).map_err(|_| ())?;
                Ok(())
            }))
            .unwrap();
    }
    pub fn get<K, F>(&mut self, key: K, callback: F)
    where
        K: Into<Vec<u8>> + Clone,
        F: FnMut(Option<Vec<u8>>) + Send + 'static,
    {
        let mut callback = Some(Box::new(callback));
        self.get_ctx(key, move |_: &mut Ctx, value| {
            callback.take().unwrap()(value)
        });
    }
    pub fn find_ctx<K, F>(&mut self, begin: K, end: K, callback: F)
    where
        K: Into<Vec<u8>> + Clone,
        F: FnMut(&mut Ctx, Vec<(Vec<u8>, Vec<u8>)>) + Send + 'static,
    {
        let begin: Vec<u8> = begin.into();
        let end: Vec<u8> = end.into();
        let mut callback: Option<Box<F>> = Some(Box::new(callback));
        self.to_worker_tx
            .send(Box::new(move |tx, db| {
                let mut callback = Some(callback.take().unwrap());
                let mut result = Some(db.find(&begin.as_slice(), &end.as_slice()).map_err(|_| ())?);
                tx.send(FromWorkerMessage::Callback(Box::new(move |ctx| {
                    callback.take().unwrap()(ctx, result.take().unwrap());
                }))).map_err(|_| ())?;
                Ok(())
            }))
            .unwrap();
    }
    pub fn find<K, F>(&mut self, begin: K, end: K, callback: F)
    where
        K: Into<Vec<u8>> + Clone,
        F: FnMut(Vec<(Vec<u8>, Vec<u8>)>) + Send + 'static,
    {
        let mut callback = Some(Box::new(callback));
        self.find_ctx(begin, end, move |_: &mut Ctx, results| {
            callback.take().unwrap()(results)
        });
    }
    pub fn find_keys_ctx<K, F>(&mut self, begin: K, end: K, callback: F)
    where
        K: Into<Vec<u8>> + Clone,
        F: FnMut(&mut Ctx, Vec<(Vec<u8>)>) + Send + 'static,
    {
        let begin: Vec<u8> = begin.into();
        let end: Vec<u8> = end.into();
        let begin: Vec<u8> = begin.into();
        let end: Vec<u8> = end.into();

        let mut callback = Some(Box::new(callback));
        self.to_worker_tx
            .send(Box::new(move |tx, db| {
                let mut result = Some(
                    db.find_keys(&begin.as_slice(), &end.as_slice())
                        .map_err(|_| ())?,
                );
                let mut callback = Some(callback.take().unwrap());
                tx.send(FromWorkerMessage::Callback(Box::new(move |ctx| {
                    callback.take().unwrap()(ctx, result.take().unwrap());
                }))).map_err(|_| ())?;
                Ok(())
            }))
            .unwrap();
    }
    pub fn find_keys<K, F>(&mut self, begin: K, end: K, mut callback: F)
    where
        K: Into<Vec<u8>> + Clone,
        F: FnMut(Vec<Vec<u8>>) + Send + 'static,
    {
        self.find_keys_ctx(begin, end, move |_, results| callback(results));
    }
    pub fn tick(&mut self) -> Box<FnMut(&mut Ctx)> {
        let mut matches = Vec::new();
        while let Ok(callback) = self.from_worker_rx.try_recv() {
            matches.push(callback);
        }
        Box::new(move |ctx: &mut Ctx| {
            for mut result in matches.drain(..) {
                match result {
                    FromWorkerMessage::Callback(mut cb) => cb(ctx),
                    _ => {}
                }
            }
        })
    }
    // wait for all pending operations to finish
    pub fn flush(&mut self) {
        self.to_worker_tx
            .send(Box::new(move |tx, _| {
                tx.send(FromWorkerMessage::Flush).map_err(|_| ())?;
                Ok(())
            }))
            .unwrap();
        while let Ok(result) = self.from_worker_rx.recv() {
            match result {
                FromWorkerMessage::Callback(_) => {}
                FromWorkerMessage::Flush => return,
            }
        }
    }
}

pub trait DBObject: serde::Serialize + serde::Deserialize<'static> {
    fn db_key() -> &'static [u8; 3];
    fn get_uuid(&self) -> Uuid;
    fn get_references(&self) -> Vec<Uuid>;
}

pub struct TypedDatabase<Ctx> {
    database: AsyncDatabase<Ctx>,
}

fn key_for_type<T: DBObject>(uuid: Uuid) -> Vec<u8> {
    T::db_key()
        .iter()
        .chain(":".as_bytes().iter())
        .chain(uuid.as_bytes().as_ref().iter())
        .cloned()
        .collect()
}

fn db_serialize<T: DBObject>(obj: &T) -> Result<Vec<u8>, ()> {
    // prepend list of object references and their count
    let serialized_obj = rmp_serde::to_vec(obj).map_err(|_| ())?;
    let mut enc = snap::Encoder::new();
    let serialized_obj = enc.compress_vec(&serialized_obj).map_err(|_|())?;
    let references = obj.get_references();
    let mut cursor = Cursor::new(vec![0; serialized_obj.len() + 4 + references.len() * 16]);

    // write the number of references
    cursor
        .write_u32::<BigEndian>(references.len() as u32)
        .map_err(|_| ())?;

    // write all references
    for r in references {
        cursor.write_all(r.as_bytes()).map_err(|_| ())?;
    }

    // write actual data
    cursor.write_all(&serialized_obj[..]).map_err(|_| ())?;
    Ok(cursor.into_inner())
}
fn db_deserialize<'a, T: DBObject>(vec: &'a [u8]) -> Result<T, ()> {
    use rmp_serde::Deserializer;
    // read header: the number of references
    if vec.len() < 4 {
        return Err(());
    }
    let num_refs = BigEndian::read_u32(&vec[0..4]);

    // skip references and read actual data (the list of references is indirectly contained in the
    // data too and is not explicitly stored in memory during use)
    let mut dec = snap::Decoder::new();
    let bytes = dec.decompress_vec(&vec[(4 + num_refs * 16) as usize..]).map_err(|_| ())?;

    let mut de = Deserializer::from_read(Cursor::new(bytes));
    serde::Deserialize::deserialize(&mut de).map_err(|_| ())
}

#[allow(dead_code)]
impl<Ctx> TypedDatabase<Ctx>
where
    Ctx: 'static,
{
    pub fn new(env: Environment) -> TypedDatabase<Ctx> {
        TypedDatabase {
            database: AsyncDatabase::<Ctx>::new(env),
        }
    }
    pub fn put<T>(&mut self, obj: &T)
    where
        T: DBObject,
    {
        let vec = db_serialize(obj);
        if let Ok(vec) = vec {
            self.database
                .put(&key_for_type::<T>(obj.get_uuid())[..], vec);
        } else {
            println!(
                "Unable to serialize object! {:?}, {}",
                T::db_key(),
                obj.get_uuid()
            )
        }
    }
    pub fn put_async<T>(&mut self, obj: T)
    where
        T: DBObject + Send + 'static,
    {
        let mut obj: Option<T> = Some(obj);
        self.database
            .to_worker_tx
            .send(Box::new(move |_, db| {
                let obj = obj.take().unwrap();
                match db_serialize(&obj) {
                    Ok(vec) => {
                        let key = key_for_type::<T>(obj.get_uuid());
                        db.put(&key.as_slice(), &vec.as_slice())
                    }
                    Err(_) => Err(()),
                }
            }))
            .unwrap();
    }
    pub fn put_many<'a, I, V, T: 'a>(&mut self, items: I) -> Result<(), ()>
    where
        I: IntoIterator<Item = V>,
        T: DBObject,
        V: Borrow<T> + Sized,
    {
        let items: Vec<(Vec<u8>, Result<Vec<u8>, ()>)> = items
            .into_iter()
            .map(|o| {
                (
                    key_for_type::<T>(o.borrow().get_uuid()).into(),
                    db_serialize(o.borrow()),
                )
            })
            .collect();

        for item in items.iter() {
            if item.1.is_err() {
                println!(
                    "Unable to serialize object! {:?}, n = {}",
                    T::db_key(),
                    items.len()
                );
                return Err(());
            }
        }
        let items: Vec<(Vec<u8>, Vec<u8>)> =
            items.into_iter().map(|x| (x.0, x.1.unwrap())).collect();

        self.database.put_many(items);
        Ok(())
    }
    pub fn put_many_async<T>(&mut self, items: Vec<T>) -> Result<(), ()>
    where
        T: DBObject + Send + 'static,
    {
        let mut items: Option<Vec<T>> = Some(items);
        self.database
            .to_worker_tx
            .send(Box::new(move |_, db| {
                {
                    let items = items.take().unwrap();
                    let items: Vec<(Vec<u8>, Result<Vec<u8>, ()>)> = items
                        .into_iter()
                        .map(|o: T| (key_for_type::<T>(o.get_uuid()).into(), db_serialize(&o)))
                        .collect();

                    for item in items.iter() {
                        if item.1.is_err() {
                            println!(
                                "Unable to serialize object! {:?}, n = {}",
                                T::db_key(),
                                items.len()
                            );
                            return Err(());
                        }
                    }
                    let items: Vec<(Vec<u8>, Vec<u8>)> =
                        items.into_iter().map(|x| (x.0, x.1.unwrap())).collect();

                    db.put_many(&items);
                }
                Ok(())
            }))
            .unwrap();
        Ok(())
    }
    pub fn put_many_async_fn<T, F>(&mut self, item_gen: F) -> Result<(), ()>
    where
        F: FnMut() -> Vec<T> + Send + 'static,
        T: DBObject + Send + 'static,
    {
        let mut item_gen = Some(item_gen);
        self.database
            .to_worker_tx
            .send(Box::new(move |_, db| {
                {
                    let mut item_gen = item_gen.take().unwrap();
                    let items = item_gen();
                    let items: Vec<(Vec<u8>, Result<Vec<u8>, ()>)> = items
                        .into_iter()
                        .map(|o: T| (key_for_type::<T>(o.get_uuid()).into(), db_serialize(&o)))
                        .collect();

                    for item in items.iter() {
                        if item.1.is_err() {
                            println!(
                                "Unable to serialize object! {:?}, n = {}",
                                T::db_key(),
                                items.len()
                            );
                            return Err(());
                        }
                    }
                    let items: Vec<(Vec<u8>, Vec<u8>)> =
                        items.into_iter().map(|x| (x.0, x.1.unwrap())).collect();

                    db.put_many(&items).unwrap();
                }
                Ok(())
            }))
            .unwrap();
        Ok(())
    }
    pub fn get_ctx<F, T: 'static>(&mut self, uuid: &[Uuid], callback: F)
    where
        T: DBObject + Send,
        F: FnMut(&mut Ctx, Vec<Option<T>>) + Send + 'static,
    {
        let keys: Vec<Vec<u8>> = uuid.iter().map(|i| key_for_type::<T>(*i)).collect();
        let mut callback: Option<Box<F>> = Some(Box::new(callback));

        self.database
            .to_worker_tx
            .send(Box::new(move |tx, db| {
                let mut callback = Some(callback.take().unwrap());

                let obj = db
                    .get_many(keys.as_slice())
                    .map_err(|_| ())?
                    .iter()
                    .map(|obj| match obj {
                        Some(bytes) => db_deserialize(bytes.as_slice()).ok(),
                        None => None,
                    })
                    .collect();

                let mut obj = Some(obj);

                tx.send(FromWorkerMessage::Callback(Box::new(move |ctx| {
                    callback.take().unwrap()(ctx, obj.take().unwrap());
                }))).map_err(|_| ())?;
                Ok(())
            }))
            .unwrap();
    }
    pub fn get<T: 'static, F>(&mut self, uuid: Uuid, mut callback: F)
    where
        T: DBObject + Send,
        F: FnMut(Option<T>) + Send + 'static,
    {
        self.get_ctx(&[uuid], move |_, object| {
            for obj in object {
                callback(obj)
            }
        });
    }

    pub fn find_all_ctx<T, F>(&mut self, callback: F)
    where
        F: FnMut(&mut Ctx, Vec<T>) + Send + 'static,
        T: DBObject,
    {
        let callback = Box::new(callback);
        let begin: Vec<u8> = T::db_key()
            .iter()
            .chain(":".as_bytes().iter())
            .cloned()
            .collect();
        let end: Vec<u8> = T::db_key()
            .iter()
            .chain(";".as_bytes().iter())
            .cloned()
            .collect();

        let mut callback = Some(callback);
        let call = move |ctx: &mut Ctx, result: Vec<(Vec<u8>, Vec<u8>)>| {
            let objects = result
                .iter()
                .filter_map(|&(ref _key, ref value)| db_deserialize(value.as_slice()).ok())
                .collect::<Vec<T>>();
            (callback.take().unwrap())(ctx, objects);
        };

        self.database.find_ctx(begin, end, call);
    }
    pub fn find_all<T, F>(&mut self, mut callback: F)
    where
        F: FnMut(Vec<T>) + Send + 'static,
        T: DBObject,
    {
        self.find_all_ctx(move |_: &mut Ctx, result| callback(result));
    }

    pub fn tick(&mut self) -> Box<FnMut(&mut Ctx)> {
        self.database.tick()
    }

    pub fn flush(&mut self) {
        self.database.flush()
    }
}

#[test]
fn test_database() {
    use tempdir::TempDir;
    let dir = TempDir::new("rampart_db_test").unwrap();
    let mut db = Database::new(Database::create_env(dir.path()));
    db.put(&"key", &"value").expect("Database::put failed");

    assert!(
        db.get(&"key",)
            .expect("Database::get failed",)
            .expect("database returned empty result",) == b"value"
    );
}

#[test]
fn test_database_simple_ctx() {
    use tempdir::TempDir;
    let dir = TempDir::new("rampart_db_test").unwrap();
    let mut db = AsyncDatabase::<bool>::new(Database::create_env(dir.path()));

    let mut done = false;

    db.put("key", "KISSA");
    db.get_ctx("key", |done, value| {
        println!("moi {:?}", value.unwrap());
        *done = true;
    });

    while !done {
        db.tick()(&mut done);
    }
}

impl DBObject for DBUnitTest {
    fn db_key() -> &'static [u8; 3] {
        b"foo"
    }
    fn get_uuid(&self) -> Uuid {
        self.uuid
    }
    fn get_references(&self) -> Vec<Uuid> {
        Vec::new()
    }
}
#[test]
fn test_async_database() {
    use tempdir::TempDir;
    let dir = TempDir::new("rampart_db_test").unwrap();

    struct Ctx {
        db: AsyncDatabase<Ctx>,
        finished: bool,
    };
    impl Ctx {
        pub fn new(path: &Path) -> Ctx {
            Ctx {
                db: AsyncDatabase::<Ctx>::new(Database::create_env(path)),
                finished: false,
            }
        }
        pub fn test(&mut self) {
            self.db.put("key", "KISSA");
            self.db
                .get_ctx("key", |context, value| context.result(value));

            while !self.finished {
                self.db.tick()(self);
            }
        }
        fn result(&mut self, value: Option<Vec<u8>>) {
            assert!(value.as_ref().unwrap() == b"KISSA");
            self.finished = true;
        }
    };

    let mut ctx = Ctx::new(dir.path());
    ctx.test();
}
#[test]
fn test_typed_database_basic() {
    use tempdir::TempDir;
    let dir = TempDir::new("rampart_db_test").unwrap();

    let mut db = TypedDatabase::<bool>::new(Database::create_env(dir.path()));
    let obj = DBUnitTest {
        data: vec![1, 2, 3],
        uuid: Uuid::nil(),
    };

    let mut done = false;
    db.get_ctx(
        &[Uuid::nil()],
        move |done: &mut bool, value: Vec<Option<DBUnitTest>>| {
            assert!(value[0].is_none());
            *done = true;
        },
    );

    while !done {
        db.tick()(&mut done);
    }
    done = false;
    while done {
        db.tick()(&mut done);
    }
    done = false;
    db.put(&obj);

    db.get_ctx(
        &[Uuid::nil()],
        move |done: &mut bool, value: Vec<Option<DBUnitTest>>| {
            let obj = value[0].as_ref().unwrap();
            assert_eq!(obj.data, vec![1, 2, 3]);
            *done = true;
        },
    );
    while !done {
        db.tick()(&mut done);
    }
}
#[test]
fn test_typed_database_advanced() {
    use tempdir::TempDir;
    let dir = TempDir::new("rampart_db_test").unwrap();

    let mut db = TypedDatabase::<bool>::new(Database::create_env(dir.path()));

    // push items, validate them from the db
    use std::collections::HashMap;

    let mut ids = HashMap::new();
    for i in 0..50 {
        let id = Uuid::new_v4();
        let data: Vec<u8> = (i..i + 10).collect();
        db.put(&DBUnitTest {
            uuid: id.clone(),
            data: data.clone(),
        });
        ids.insert(id, data);
    }

    // get items from db
    db.find_all_ctx(move |done, list: Vec<DBUnitTest>| {
        assert_eq!(list.len(), 50);
        for item in list.iter() {
            let orig_data = ids.get(&item.uuid).unwrap();
            assert_eq!(item.data, *orig_data);
            println!("{}, {:?}", &item.uuid, orig_data);
        }
        *done = true;
    });

    // START
    let mut done = false;
    while !done {
        db.tick()(&mut done);
    }
}

#[test]
fn test_simultaneous_database_access() {
    use tempdir::TempDir;
    let dir = TempDir::new("rampart_db_test").unwrap();
    let env = Database::create_env(dir.path());
    let mut db1 = Database::new(env.clone());
    let mut db2 = Database::new(env);

    db1.put(&"foo", &"bar").unwrap();
    db2.put(&"kissa", &"koira").unwrap();

    assert_eq!(
        db2.get(&"foo").expect("DB ERROR").expect("VALUE MISSING"),
        b"bar"
    );
    assert_eq!(
        db1.get(&"kissa").expect("DB ERROR").expect("VALUE MISSING"),
        b"koira"
    );
}
