use super::*;
use crate::servers::connection::Connection;
use russh_sftp::client::SftpSession;
use std::{io::SeekFrom, time::Duration};
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};

pub fn download(
    connection: &Connection,
    mut coordinator: Coordinator,
) -> Result<TransferReceipt, TransferError> {
    connection.verify_transfer_source(&coordinator.receipt.source)?;
    let cancellation = coordinator.cancellation.clone();
    let operation = async {
        let channel = connection
            .transfer_session()
            .channel_open_session()
            .await
            .map_err(|_| TransferError::SftpUnavailable)?;
        channel
            .request_subsystem(true, "sftp")
            .await
            .map_err(|_| TransferError::SftpUnavailable)?;
        let sftp = tokio::time::timeout(
            Duration::from_secs(15),
            SftpSession::new(channel.into_stream()),
        )
        .await
        .map_err(|_| TransferError::Timeout)?
        .map_err(|_| TransferError::SftpUnavailable)?;
        sftp.set_timeout(30);
        let source = coordinator.receipt.source.clone();
        let metadata = sftp
            .symlink_metadata(&source.remote_path)
            .await
            .map_err(|_| TransferError::SourceChanged)?;
        if !metadata.file_type().is_file()
            || metadata.size != Some(source.size)
            || metadata.mtime != Some(source.modified)
        {
            return Err(TransferError::SourceChanged);
        }
        let mut file = sftp
            .open(&source.remote_path)
            .await
            .map_err(|_| TransferError::SourceChanged)?;
        file.seek(SeekFrom::Start(coordinator.receipt.bytes))
            .await?;
        coordinator.state(TransferState::Transferring)?;
        let mut buffer = [0u8; 65536];
        loop {
            let cancellation = coordinator.cancellation.clone();
            let n = tokio::select! {
                _=cancellation.cancelled()=>return Err(TransferError::Cancelled),
                result=tokio::time::timeout(Duration::from_secs(30),file.read(&mut buffer))=>result.map_err(|_|TransferError::Timeout)??,
            };
            if n == 0 {
                break;
            }
            coordinator.write(&buffer[..n])?;
        }
        let metadata = sftp
            .symlink_metadata(&source.remote_path)
            .await
            .map_err(|_| TransferError::SourceChanged)?;
        if !metadata.file_type().is_file()
            || metadata.size != Some(source.size)
            || metadata.mtime != Some(source.modified)
        {
            return Err(TransferError::SourceChanged);
        }
        let _ = file.shutdown().await;
        let _ = sftp.close().await;
        Ok(())
    };
    let result = connection.transfer_runtime().block_on(async { tokio::select! {
        _=cancellation.cancelled()=>Err(TransferError::Cancelled),
        result=tokio::time::timeout(Duration::from_secs(21600),operation)=>result.map_err(|_|TransferError::Timeout)?,
    }});
    if let Err(error) = result {
        coordinator.fail(error);
        return Err(error);
    }
    coordinator.finish()
}
