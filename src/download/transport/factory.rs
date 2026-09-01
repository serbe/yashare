use std::path::Path;

use crate::{
    CHUNK_SIZE,
    api::HttpClient,
    download::{DownloadContext, resume::ResumeManager, transport::session::DownloadSession},
    fs::{ChecksumSpec, FileVerifier, VerificationMode},
};

/// Владеет состоянием, нужным для скачивания файлов одним воркером:
/// хендлом HTTP-клиента, учётом резюма (`ResumeManager`) и верификатором
/// контрольных сумм. По запросу выдаёт короткоживущий `DownloadSession`,
/// заимствующий из `self`.
pub(crate) struct SessionFactory {
    http: HttpClient,
    resume: ResumeManager,
    verifier: FileVerifier,
    verify_mode: VerificationMode,
}

impl SessionFactory {
    /// Создаёт фабрику из общего контекста скачивания.
    pub(crate) fn new(ctx: &DownloadContext) -> Self {
        Self {
            http: ctx.http.clone(),
            resume: ResumeManager::new(),
            verifier: FileVerifier::new(CHUNK_SIZE),
            verify_mode: ctx.verify_mode,
        }
    }

    /// Заимствует `DownloadSession` для одной попытки скачивания.
    ///
    /// Сессия занимает `&mut self` (из-за верификатора), поэтому активна
    /// только одна за раз — это ровно то, как ей пользуется `DownloadWorker`:
    /// один job, полностью дожидаемся, потом следующий.
    pub(crate) fn session(&mut self) -> DownloadSession<'_> {
        DownloadSession::new(&self.http, &self.resume, &mut self.verifier, self.verify_mode)
    }

    pub(crate) async fn file_matches(
        &mut self,
        destination: &Path,
        expected_size: u64,
        checksum: &ChecksumSpec,
    ) -> std::io::Result<bool> {
        self.verifier
            .file_matches(destination, expected_size, checksum, self.verify_mode)
            .await
    }
}
