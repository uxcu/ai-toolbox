import React, { useCallback, useState } from 'react';
import { Modal, Button, Checkbox, Alert, Spin, message } from 'antd';
import { useTranslation } from 'react-i18next';
import { open as openDialog } from '@tauri-apps/plugin-dialog';
import {
  importCcSwitchConfig,
  previewCcSwitchConfig,
  ImportOptions,
  CcSwitchPreview,
  CcSwitchImportResult,
} from '@/services/ccSwitchApi';

interface CcSwitchImportModalProps {
  open: boolean;
  onClose: () => void;
  onSuccess?: () => void;
}

export const CcSwitchImportModal: React.FC<CcSwitchImportModalProps> = ({
  open,
  onClose,
  onSuccess,
}) => {
  const { t } = useTranslation();

  const [selectedFile, setSelectedFile] = useState<string>('');

  const [preview, setPreview] = useState<CcSwitchPreview | null>(null);
  const [previewLoading, setPreviewLoading] = useState(false);

  const [importOptions, setImportOptions] = useState<ImportOptions>({
    importClaude: true,
    importCodex: true,
    importMcp: true,
    skipDuplicates: true,
  });

  const [importLoading, setImportLoading] = useState(false);
  const [importResult, setImportResult] = useState<CcSwitchImportResult | null>(null);

  const [error, setError] = useState<string | null>(null);

  const handleSelectFile = useCallback(async () => {
    try {
      const filePath = await openDialog({
        filters: [{ name: 'JSON', extensions: ['json'] }],
        multiple: false,
      });

      if (filePath && typeof filePath === 'string') {
        setSelectedFile(filePath);
        setPreview(null);
        setImportResult(null);
        setError(null);

        await handlePreview(filePath);
      }
    } catch (err) {
      console.error('Failed to open file dialog:', err);
      setError(t('ccswitch.error.fileDialog'));
    }
  }, [t]);

  const handlePreview = async (filePath: string) => {
    setPreviewLoading(true);
    setError(null);

    try {
      const previewData = await previewCcSwitchConfig(filePath);
      setPreview(previewData);

      if (previewData.error) {
        setError(previewData.error);
      }
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      setError(t('ccswitch.error.previewFailed', { message: errorMessage }));
    } finally {
      setPreviewLoading(false);
    }
  };

  const handleManualPreview = useCallback(async () => {
    if (!selectedFile) {
      message.warning(t('ccswitch.warning.selectFileFirst'));
      return;
    }
    await handlePreview(selectedFile);
  }, [selectedFile, t]);

  const handleImport = async () => {
    if (!selectedFile) {
      message.warning(t('ccswitch.warning.selectFileFirst'));
      return;
    }

    if (!preview?.canImport) {
      message.warning(t('ccswitch.warning.cannotImport'));
      return;
    }

    if (!importOptions.importClaude && !importOptions.importCodex && !importOptions.importMcp) {
      message.warning(t('ccswitch.warning.selectImportType'));
      return;
    }

    setImportLoading(true);
    setError(null);

    try {
      const result = await importCcSwitchConfig(selectedFile, importOptions);
      setImportResult(result);

      if (result.success) {
        message.success(t('ccswitch.importSuccess'));
        onSuccess?.();
      } else if (result.errors.length > 0) {
        message.warning(t('ccswitch.importPartialSuccess'));
      }
    } catch (err) {
      const errorMessage = err instanceof Error ? err.message : String(err);
      setError(t('ccswitch.error.importFailed', { message: errorMessage }));
    } finally {
      setImportLoading(false);
    }
  };

  const handleClose = useCallback(() => {
    setSelectedFile('');
    setPreview(null);
    setImportResult(null);
    setError(null);
    setImportOptions({
      importClaude: true,
      importCodex: true,
      importMcp: true,
      skipDuplicates: true,
    });
    onClose();
  }, [onClose]);

  const canImport = preview?.canImport && !importLoading && !previewLoading;

  return (
    <Modal
      title={t('ccswitch.importTitle')}
      open={open}
      onCancel={handleClose}
      footer={null}
      width={600}
      closable={!importLoading}
      maskClosable={!importLoading}
    >
      <Spin spinning={previewLoading || importLoading}>
        <div style={{ marginBottom: 16 }}>
          <Button onClick={handleSelectFile} disabled={importLoading}>
            {t('ccswitch.selectFile')}
          </Button>
          {selectedFile && (
            <div style={{ marginTop: 8, wordBreak: 'break-all', color: '#666' }}>
              {selectedFile}
            </div>
          )}
        </div>

        {selectedFile && !preview && !previewLoading && (
          <div style={{ marginBottom: 16 }}>
            <Button onClick={handleManualPreview} disabled={!selectedFile}>
              {t('ccswitch.preview')}
            </Button>
          </div>
        )}

        {error && (
          <Alert
            type="error"
            message={t('ccswitch.errorTitle')}
            description={error}
            style={{ marginBottom: 16 }}
            closable
            onClose={() => setError(null)}
          />
        )}

        {preview && !preview.error && (
          <div style={{ marginBottom: 16 }}>
            <Alert
              type="info"
              message={t('ccswitch.previewTitle')}
              description={
                <div>
                  <div>{t('ccswitch.version', { version: preview.version })}</div>
                  <div style={{ marginTop: 8 }}>
                    <div>
                      {t('ccswitch.claudeProviders', {
                        count: preview.providerCounts.claude,
                      })}
                    </div>
                    <div>
                      {t('ccswitch.codexProviders', {
                        count: preview.providerCounts.codex,
                      })}
                    </div>
                    <div>
                      {t('ccswitch.mcpServers', { count: preview.mcpCount })}
                    </div>
                  </div>
                  {!preview.canImport && (
                    <div style={{ marginTop: 8, color: '#faad14' }}>
                      {t('ccswitch.versionIncompatible')}
                    </div>
                  )}
                </div>
              }
            />
          </div>
        )}

        {preview && preview.canImport && !importResult && (
          <div style={{ marginBottom: 16 }}>
            <div style={{ marginBottom: 8, fontWeight: 500 }}>
              {t('ccswitch.importOptions')}
            </div>
            <Checkbox
              checked={importOptions.importClaude}
              onChange={(e) =>
                setImportOptions((prev) => ({
                  ...prev,
                  importClaude: e.target.checked,
                }))
              }
            >
              {t('ccswitch.importClaudeProviders')}
            </Checkbox>
            <br />
            <Checkbox
              checked={importOptions.importCodex}
              onChange={(e) =>
                setImportOptions((prev) => ({
                  ...prev,
                  importCodex: e.target.checked,
                }))
              }
            >
              {t('ccswitch.importCodexProviders')}
            </Checkbox>
            <br />
            <Checkbox
              checked={importOptions.importMcp}
              onChange={(e) =>
                setImportOptions((prev) => ({
                  ...prev,
                  importMcp: e.target.checked,
                }))
              }
            >
              {t('ccswitch.importMcpServers')}
            </Checkbox>
            <br />
            <Checkbox
              checked={importOptions.skipDuplicates}
              onChange={(e) =>
                setImportOptions((prev) => ({
                  ...prev,
                  skipDuplicates: e.target.checked,
                }))
              }
            >
              {t('ccswitch.skipDuplicates')}
            </Checkbox>
          </div>
        )}

        {importResult && (
          <div style={{ marginBottom: 16 }}>
            {importResult.success ? (
              <Alert
                type="success"
                message={t('ccswitch.importSuccess')}
                description={
                  <div>
                    <div>
                      {t('ccswitch.claudeImported', {
                        count: importResult.providersImported.claude,
                      })}
                    </div>
                    <div>
                      {t('ccswitch.codexImported', {
                        count: importResult.providersImported.codex,
                      })}
                    </div>
                    <div>
                      {t('ccswitch.mcpImported', {
                        count: importResult.mcpServersImported,
                      })}
                    </div>
                    {importResult.errors.length > 0 && (
                      <div style={{ marginTop: 8 }}>
                        <Alert
                          type="warning"
                          message={t('ccswitch.importErrors')}
                          description={
                            <ul style={{ margin: 0, paddingLeft: 16 }}>
                              {importResult.errors.map((err, index) => (
                                <li key={index}>{err}</li>
                              ))}
                            </ul>
                          }
                        />
                      </div>
                    )}
                  </div>
                }
              />
            ) : (
              <Alert
                type="error"
                message={t('ccswitch.importFailed')}
                description={importResult.message || t('ccswitch.error.unknown')}
              />
            )}
          </div>
        )}

        <div style={{ display: 'flex', justifyContent: 'flex-end', gap: 8 }}>
          <Button onClick={handleClose} disabled={importLoading}>
            {t('common.cancel')}
          </Button>
          {importResult ? (
            <Button type="primary" onClick={handleClose}>
              {t('common.close')}
            </Button>
          ) : (
            <Button
              type="primary"
              onClick={handleImport}
              disabled={!canImport || (!importOptions.importClaude && !importOptions.importCodex && !importOptions.importMcp)}
              loading={importLoading}
            >
              {t('ccswitch.import')}
            </Button>
          )}
        </div>
      </Spin>
    </Modal>
  );
};

export default CcSwitchImportModal;
