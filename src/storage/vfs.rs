use std::fmt::Debug;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use async_trait::async_trait;
use futures_util::StreamExt;
use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};
use vfs::async_vfs::AsyncFileSystem;
use vfs::VfsFileType;

use crate::auth::UserDetail;
use crate::storage::{ErrorKind, Fileinfo, Metadata, Permissions, StorageBackend};

fn normalize_path(path: &Path) -> PathBuf {
    if path.starts_with("/") {
        path.to_path_buf()
    }else{
        let mut path_buf = PathBuf::new();
        path_buf.push("/");
        path_buf.push(path);
        path_buf
    }
}
/// VfsStorageBackend
#[derive(Debug,Clone)]
pub struct VfsStorageBackend<VFS> {
    vfs: Arc<VFS>,
    // path: Path
}

impl<VFS> VfsStorageBackend<VFS> {
    ///
    pub fn new(vfs: Arc<VFS>)->Self {
        Self{
            vfs
        }
    }
}

#[async_trait]
impl<User: UserDetail,VFS: AsyncFileSystem> StorageBackend<User> for VfsStorageBackend<VFS> {
    type Metadata = vfs::VfsMetadata;

    fn enter(&mut self, _user_detail: &User) -> io::Result<()> {
        // if let Some(path) = user_detail.home() {
        //     let relpath = match path.strip_prefix(self.root.as_path()) {
        //         Ok(r) => r,
        //         Err(_) => return Err(io::Error::new(io::ErrorKind::Other, "Path not a descendant of the previous root")),
        //     };
        //     self.root_fd = Arc::new(self.root_fd.open_dir(relpath)?);
        // }
        Ok(())
    }

    fn supported_features(&self) -> u32 {
        crate::storage::FEATURE_RESTART | crate::storage::FEATURE_SITEMD5
    }

    #[tracing_attributes::instrument]
    async fn metadata<P: AsRef<Path> + Send + Debug>(&self, _user: &User, path: P) -> crate::storage::Result<Self::Metadata> {
        let path = normalize_path(path.as_ref());
        Ok(self.vfs.metadata(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))?)
    }

    #[allow(clippy::type_complexity)]
    #[tracing_attributes::instrument]
    async fn list<P>(&self, _user: &User, path: P) -> crate::storage::Result<Vec<Fileinfo<std::path::PathBuf, Self::Metadata>>>
        where
            P: AsRef<Path> + Send + Debug,
            <Self as StorageBackend<User>>::Metadata: Metadata,
    {

        use futures_util::stream::TryStreamExt;
        let path = normalize_path(path.as_ref());

        let stream = self.vfs.read_dir(path.to_str().unwrap()).await
            .map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))?;
        let fis: Vec<Fileinfo<std::path::PathBuf, Self::Metadata>> =TryStreamExt::try_collect(stream
            .then(|n| async move{
                let metadata = self.vfs.metadata(n.as_str()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))?;
                Ok::<_, crate::storage::Error>(Fileinfo{
                    path: PathBuf::from(n),
                    metadata,
                })
            })
        ).await?;

        Ok(fis)
    }

    //#[tracing_attributes::instrument]
    async fn get<P: AsRef<Path> + Send + Debug>(&self, _user: &User, path: P, start_pos: u64) -> crate::storage::Result<Box<dyn tokio::io::AsyncRead + Send + Sync + Unpin>> {
        let path = normalize_path(path.as_ref());
        use futures_util::AsyncSeekExt;
        let mut reader = self.vfs.open_file(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))?;
        if start_pos > 0 {
            reader.seek(std::io::SeekFrom::Start(start_pos)).await?;
        }

        let (local,mut remote) = tokio::io::duplex(4096);

        tokio::spawn(async move {
            let mut reader = reader.compat();
            let _ = tokio::io::copy(&mut reader,&mut remote).await;
        });

        Ok(Box::new(local) as Box<dyn tokio::io::AsyncRead + Send + Sync + Unpin>)
    }

    async fn put<P: AsRef<Path> + Send, R: tokio::io::AsyncRead + Send + Sync + 'static + Unpin>(
        &self,
        _user: &User,
        mut bytes: R,
        path: P,
        start_pos: u64,
    ) -> crate::storage::Result<u64> {
        let path = normalize_path(path.as_ref());
        let writer = self.vfs.create_file(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError, err))?;
        if start_pos >0 {
            unimplemented!()
        }
        let mut writer = Box::into_pin(writer).compat_write();
        // let mut writer = Box::into_pin(writer);
        let bytes_copied = tokio::io::copy(&mut bytes, &mut writer).await?;
        Ok(bytes_copied)
    }

    #[tracing_attributes::instrument]
    async fn del<P: AsRef<Path> + Send + Debug>(&self, _user: &User, path: P) -> crate::storage::Result<()> {
        let path = normalize_path(path.as_ref());
        self.vfs.remove_file(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))
    }

    #[tracing_attributes::instrument]
    async fn mkd<P: AsRef<Path> + Send + Debug>(&self, _user: &User, path: P) -> crate::storage::Result<()> {
        let path = normalize_path(path.as_ref());
        self.vfs.create_dir(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))
    }

    #[tracing_attributes::instrument]
    async fn rename<P: AsRef<Path> + Send + Debug>(&self, _user: &User, from: P, to: P) -> crate::storage::Result<()> {
        let from = normalize_path(from.as_ref());
        let to= normalize_path(to.as_ref());
        self.vfs.move_file(from.to_str().unwrap(),to.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))
    }

    #[tracing_attributes::instrument]
    async fn rmd<P: AsRef<Path> + Send + Debug>(&self, _user: &User, path: P) -> crate::storage::Result<()> {
        let path = normalize_path(path.as_ref());
        self.vfs.remove_dir(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))
    }

    #[tracing_attributes::instrument]
    async fn cwd<P: AsRef<Path> + Send + Debug>(&self, user: &User, path: P) -> crate::storage::Result<()> {
        let _path = normalize_path(path.as_ref());
        // unimplemented!()
        // self.list(user, path).await.map(drop)
        Ok(())
    }
}

impl Metadata for vfs::VfsMetadata {
    fn len(&self) -> u64 {
        self.len
    }

    fn is_dir(&self) -> bool {
        self.file_type == VfsFileType::Directory
    }

    fn is_file(&self) -> bool {
        self.file_type == VfsFileType::File
    }

    fn is_symlink(&self) -> bool {
        // unimplemented!()
        false
    }

    fn modified(&self) -> crate::storage::Result<SystemTime> {
        Ok(self.modified.unwrap_or_else(||SystemTime::now()))
    }

    fn gid(&self) -> u32 {
        0
    }

    fn uid(&self) -> u32 {
        0
    }

    fn links(&self) -> u64 {
        1
    }

    fn permissions(&self) -> Permissions {
        Permissions(0o7755)
    }
}



#[cfg(test)]
mod test {
    use std::io::SeekFrom;
    use std::net::SocketAddr;
    use std::sync::Arc;

    use async_ftp::FtpStream;
    use futures_util::{AsyncSeekExt, AsyncWriteExt};
    use vfs::async_vfs::{AsyncFileSystem, AsyncMemoryFS};
    use vfs::VfsResult;

    use crate::ServerBuilder;
    use crate::storage::VfsStorageBackend;

    #[tokio::test]
    async fn copy_file_across_filesystems() -> VfsResult<()> {
        let vfs = AsyncMemoryFS::new();
        {
            let mut writer = vfs.create_file("/test.txt").await.unwrap();
            writer.write_all(b"test.txt").await.unwrap();
            writer.flush().await.unwrap();
            writer.close().await.unwrap();
        }
        {

            use futures_util::AsyncReadExt;
            let mut reader  = vfs.open_file("/test.txt").await.unwrap();
            reader.seek(SeekFrom::Start(0)).await.unwrap();
            let mut r = String::new();
            reader.read_to_string(&mut r).await.unwrap();
            println!("rrrs {}",r);
        }
        Ok(())
    }
    async fn server_listen()->SocketAddr
    {
        let addr = SocketAddr::from(([127,0,0,1],1234));

        tokio::spawn(async move {
            let vfs = Arc::new(AsyncMemoryFS::new());
            {
                let mut writer = vfs.create_file("/test.txt").await.unwrap();
                writer.write_all(b"test.txt").await.unwrap();
                writer.flush().await.unwrap();
                writer.close().await.unwrap();
            }
            let server = ServerBuilder::new(Box::new(move || VfsStorageBackend::new(vfs.clone()))).build().unwrap();

            server.listen(addr.to_string()).await.unwrap();
        });
        while async_ftp::FtpStream::connect(&addr).await.is_err() {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        addr
    }


    #[tokio::test]
    async fn test() {
        println!("test");
        let addr = server_listen().await;

        // Retrieve the remote file
        let mut ftp_stream = FtpStream::connect(addr).await.unwrap();

        ftp_stream.login("hoi", "jij").await.unwrap();


        let remote_file = ftp_stream.simple_retr("test.txt").await.unwrap().into_inner();
        pretty_assertions::assert_eq!(remote_file,b"test.txt");

        assert_eq!(ftp_stream.simple_retr("bla.txt").await.is_err(),true);


        let mut data = vec![0; 1024];
        getrandom::getrandom(&mut data).expect("Error generating random bytes");

        ftp_stream.put("bla.txt", &mut &data[..]).await.unwrap();
        let remote_file = ftp_stream.simple_retr("bla.txt").await.unwrap();
        let remote_data = remote_file.into_inner();

        pretty_assertions::assert_eq!(remote_data, data);

        ftp_stream.rm("bla.txt").await.unwrap();

        assert_eq!(ftp_stream.simple_retr("bla.txt").await.is_err(),true);
    }
}
