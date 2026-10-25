//! CSAFE-STREAM/1: exact ASCII header + length-delimited binary + exact trailer,
//! followed by successful SSH exit status and channel completion. No UTF-8 payload API.
use super::*;
use crate::servers::connection::Connection;
use russh::ChannelMsg;
use std::time::Duration;

fn header(receipt: &TransferReceipt) -> Vec<u8> {
    format!(
        "CSAFE-STREAM/1\n{}\n{}\n{}\n{}\nrestic-repository-tar-v1\nencrypted\n\n",
        receipt.id, receipt.source.artifact_id, receipt.source.size, receipt.source.sha256
    )
    .into_bytes()
}
fn trailer(receipt: &TransferReceipt) -> Vec<u8> {
    format!("\nCSAFE-END/1 {} {}\n", receipt.id, receipt.source.sha256).into_bytes()
}

struct Framing {
    header: Vec<u8>,
    trailer: Vec<u8>,
    head: usize,
    tail: usize,
}
impl Framing {
    fn new(receipt: &TransferReceipt) -> Self {
        Self {
            header: header(receipt),
            trailer: trailer(receipt),
            head: 0,
            tail: 0,
        }
    }
    fn receive(&mut self, mut data: &[u8], job: &mut Coordinator) -> Result<(), TransferError> {
        if self.head < self.header.len() {
            let n = data.len().min(self.header.len() - self.head);
            if data[..n] != self.header[self.head..self.head + n] {
                return Err(TransferError::InvalidReceipt);
            }
            self.head += n;
            data = &data[n..];
        }
        if self.head == self.header.len() && job.receipt.bytes < job.receipt.source.size {
            let n = (job.receipt.source.size - job.receipt.bytes).min(data.len() as u64) as usize;
            job.write(&data[..n])?;
            data = &data[n..];
        }
        if !data.is_empty() {
            if self.tail + data.len() > self.trailer.len()
                || data != &self.trailer[self.tail..self.tail + data.len()]
            {
                return Err(TransferError::Incomplete);
            }
            self.tail += data.len();
        }
        Ok(())
    }
    fn complete(&self) -> bool {
        self.head == self.header.len() && self.tail == self.trailer.len()
    }
}
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn receive(
    connection: &Connection,
    mut job: Coordinator,
) -> Result<TransferReceipt, TransferError> {
    connection.verify_transfer_source(&job.receipt.source)?;
    if job.receipt.kind != TransferKind::SshStream || job.receipt.bytes != 0 {
        return Err(TransferError::InvalidReceipt);
    }
    job.state(TransferState::Connecting)?;
    let source = job.receipt.source.clone();
    let command = format!(
        "sh -s -- {} {} {} {} {} {}",
        quote(&source.remote_path),
        source.size,
        source.modified,
        job.receipt.id,
        source.artifact_id,
        source.sha256
    );
    let cancellation = job.cancellation.clone();
    let result=connection.transfer_runtime().block_on(async {
        let operation=async {
            let mut channel=connection.transfer_session().channel_open_session().await.map_err(|_|TransferError::RemoteFailed)?;
            let task=async {
                channel.exec(true,command).await.map_err(|_|TransferError::RemoteFailed)?;
                channel.data(include_bytes!("stream_artifact.sh").as_slice()).await.map_err(|_|TransferError::RemoteFailed)?;
                channel.eof().await.map_err(|_|TransferError::RemoteFailed)?;
                job.state(TransferState::Transferring)?;
                let mut framing=Framing::new(&job.receipt);let mut status=None;let mut stderr=0usize;
                loop {
                    let cancellation=job.cancellation.clone();
                    let message=tokio::select! {
                        _=cancellation.cancelled()=>return Err(TransferError::Cancelled),
                        message=tokio::time::timeout(Duration::from_secs(30),channel.wait())=>message.map_err(|_|TransferError::Timeout)?,
                    };
                    let Some(message)=message else{break;};
                    match message {
                        ChannelMsg::Data{data}=>framing.receive(&data,&mut job)?,
                        ChannelMsg::ExtendedData{data,..}=>{stderr=stderr.saturating_add(data.len());if stderr>65536{return Err(TransferError::RemoteFailed);}},
                        ChannelMsg::ExitStatus{exit_status}=>status=Some(exit_status),
                        ChannelMsg::ExitSignal{..}=>return Err(TransferError::RemoteFailed),
                        _=>{},
                    }
                }
                if status!=Some(0) {return Err(TransferError::RemoteFailed);}
                if !framing.complete(){return Err(TransferError::Incomplete);}Ok(())
            }.await;
            if task.is_err() {let _=tokio::time::timeout(Duration::from_secs(1),channel.signal(russh::Sig::TERM)).await;}
            let _=tokio::time::timeout(Duration::from_secs(1),channel.close()).await;task
        };
        tokio::select! {
            _=cancellation.cancelled()=>Err(TransferError::Cancelled),
            result=tokio::time::timeout(Duration::from_secs(21600),operation)=>result.map_err(|_|TransferError::Timeout)?,
        }
    });
    if let Err(error) = result {
        job.fail(error);
        return Err(error);
    }
    job.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sha2::{Digest, Sha256};
    use std::sync::Arc;
    use uuid::Uuid;
    #[test]
    fn every_framing_boundary_and_missing_marker_is_checked() {
        let data = b"binary\0\xff\r\nfixture";
        let id = Uuid::new_v4();
        let source = SourceIdentity {
            server_id: Uuid::new_v4(),
            endpoint: "fixture".into(),
            host_key: "pinned".into(),
            artifact_id: id,
            snapshot_id: "a".repeat(64),
            remote_path: format!("/exports/{id}.csbackup.tar"),
            size: data.len() as u64,
            modified: 1,
            sha256: format!("{:x}", Sha256::digest(data)),
        };
        for chunk in [1, 7, 65536] {
            let directory = tempfile::tempdir().unwrap();
            let receipt = TransferReceipt::new(
                source.clone(),
                directory.path().into(),
                TransferKind::SshStream,
            )
            .unwrap();
            let mut framing = Framing::new(&receipt);
            let mut bytes = header(&receipt);
            bytes.extend(data);
            bytes.extend(trailer(&receipt));
            let mut job = Coordinator::open(
                receipt,
                &directory.path().join("r.json"),
                Arc::new(|_| {}),
                CancellationToken::default(),
            )
            .unwrap();
            for part in bytes.chunks(chunk) {
                framing.receive(part, &mut job).unwrap();
            }
            assert!(framing.complete());
            assert!(job.finish().unwrap().sha256_verified);
        }
        let directory = tempfile::tempdir().unwrap();
        let receipt =
            TransferReceipt::new(source, directory.path().into(), TransferKind::SshStream).unwrap();
        let mut framing = Framing::new(&receipt);
        let mut bytes = header(&receipt);
        bytes.extend(data);
        let mut job = Coordinator::open(
            receipt,
            &directory.path().join("r.json"),
            Arc::new(|_| {}),
            CancellationToken::default(),
        )
        .unwrap();
        framing.receive(&bytes, &mut job).unwrap();
        assert!(!framing.complete());
    }
}
