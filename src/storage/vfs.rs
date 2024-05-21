use std::fmt::Debug;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use async_trait::async_trait;
use futures_util::StreamExt;
use tokio_util::compat::{FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt};
use vfs::async_vfs::AsyncFileSystem;
use vfs::VfsFileType;

use crate::auth::UserDetail;
use crate::storage::{ErrorKind, Fileinfo, Metadata, Permissions, StorageBackend};

fn strip_prefixes(path: &Path) -> PathBuf {
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
#[derive(Debug)]
pub struct VfsStorageBackend<VFS> {
    vfs: VFS,
    // path: Path
}

impl<VFS> VfsStorageBackend<VFS> {
    ///
    pub fn new(vfs: VFS)->Self {
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
        let path = strip_prefixes(path.as_ref());
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
        let path = strip_prefixes(path.as_ref());

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
        let path = strip_prefixes(path.as_ref());
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
        let path = strip_prefixes(path.as_ref());
        let writer = self.vfs.append_file(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError, err))?;
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
        let path = strip_prefixes(path.as_ref());
        self.vfs.remove_file(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))
    }

    #[tracing_attributes::instrument]
    async fn mkd<P: AsRef<Path> + Send + Debug>(&self, _user: &User, path: P) -> crate::storage::Result<()> {
        let path = strip_prefixes(path.as_ref());
        self.vfs.create_dir(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))
    }

    #[tracing_attributes::instrument]
    async fn rename<P: AsRef<Path> + Send + Debug>(&self, _user: &User, from: P, to: P) -> crate::storage::Result<()> {
        let from = strip_prefixes(from.as_ref());
        let to= strip_prefixes(to.as_ref());
        self.vfs.move_file(from.to_str().unwrap(),to.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))
    }

    #[tracing_attributes::instrument]
    async fn rmd<P: AsRef<Path> + Send + Debug>(&self, _user: &User, path: P) -> crate::storage::Result<()> {
        let path = strip_prefixes(path.as_ref());
        self.vfs.remove_dir(path.to_str().unwrap()).await.map_err(|err| crate::storage::Error::new(ErrorKind::LocalError,err))
    }

    #[tracing_attributes::instrument]
    async fn cwd<P: AsRef<Path> + Send + Debug>(&self, user: &User, path: P) -> crate::storage::Result<()> {
        let path = strip_prefixes(path.as_ref());
        self.list(user, path).await.map(drop)
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
