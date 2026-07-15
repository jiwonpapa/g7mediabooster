<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use JsonException;
use Throwable;
use UnexpectedValueException;

final class ApiEndpoint
{
    private const MAX_JSON_BYTES = 1024 * 1024;

    public function __construct(
        private readonly GnuboardRuntime $runtime,
        private readonly BatchValidator $batchValidator = new BatchValidator,
        private readonly ReadyAssetValidator $readyValidator = new ReadyAssetValidator,
    ) {}

    public function run(): never
    {
        try {
            $action = is_string($_GET['action'] ?? null) ? $_GET['action'] : '';
            $boardTable = is_string($_GET['bo_table'] ?? null) ? $_GET['bo_table'] : '';
            $board = $this->runtime->board($boardTable);
            $this->runtime->assertUploadPermission($board);
            $configuration = $this->runtime->configuration();
            if ($action === 'configuration' && $this->method() === 'GET') {
                $this->success([
                    'enabled' => $configuration->enabled,
                    'max_files' => min(100, max(1, (int) $board['bo_upload_count'])),
                    'max_file_size_bytes' => max(1, (int) $board['bo_upload_size']),
                    'max_parallel_files' => $configuration->maxParallelFiles,
                    'max_parallel_parts' => $configuration->maxParallelParts,
                    'max_part_retries' => $configuration->maxPartRetries,
                    'status_poll_interval_ms' => $configuration->statusPollIntervalMs,
                ]);
            }
            if (! $configuration->enabled) {
                throw new HttpFailure(503, 'ADAPTER_DISABLED', '미디어 부스터가 비활성화되어 있습니다.');
            }
            $this->runtime->assertCsrf();
            $ownerKey = $this->runtime->ownerKey();
            $store = $this->runtime->store();
            $client = $this->runtime->client();

            match ($action) {
                'batch' => $this->createBatch($board, $boardTable, $ownerKey, $store, $client),
                'presign-part' => $this->presignPart($boardTable, $ownerKey, $store, $client),
                'complete-multipart' => $this->completeMultipart($boardTable, $ownerKey, $store, $client),
                'abort-multipart' => $this->abortMultipart($boardTable, $ownerKey, $store, $client),
                'confirm-single' => $this->confirmSingle($boardTable, $ownerKey, $store, $client),
                'status' => $this->status($boardTable, $ownerKey, $store, $client),
                'prepare' => $this->prepare($boardTable, $ownerKey, $store, $client),
                'delete' => $this->delete($boardTable, $ownerKey, $store, $client),
                default => throw new HttpFailure(404, 'ROUTE_NOT_FOUND', '업로드 요청 경로를 찾을 수 없습니다.'),
            };
        } catch (HttpFailure $error) {
            $this->failure($error->status, $error->errorCode, $error->getMessage());
        } catch (UpstreamException $error) {
            $this->failure($error->httpStatus, $error->errorCode, $error->getMessage(), $error->requestId);
        } catch (UnexpectedValueException) {
            $this->failure(422, 'INVALID_UPLOAD_REQUEST', '업로드 요청 또는 처리 결과가 올바르지 않습니다.');
        } catch (Throwable $error) {
            error_log('G7MediaBooster G5 API failure: '.get_class($error));
            $this->failure(503, 'ADAPTER_UNAVAILABLE', '미디어 업로드 제어 기능을 사용할 수 없습니다.');
        }
    }

    /** @param array<string, mixed> $board */
    private function createBatch(
        array $board,
        string $boardTable,
        string $ownerKey,
        SessionStore $store,
        ControlClient $client,
    ): never {
        $this->requireMethod('POST');
        $body = $this->jsonBody();
        $files = $this->batchValidator->validateRequest(
            $body['files'] ?? null,
            (int) $board['bo_upload_count'],
            (int) $board['bo_upload_size'],
        );
        $controlFiles = array_map(static function (array $file): array {
            unset($file['original_filename']);

            return $file;
        }, $files);
        $batch = $this->batchValidator->validateResponse(
            $client->createBatch(['files' => $controlFiles]),
            $files,
        );
        $store->recordBatch($ownerKey, $boardTable, $files, $batch);

        $this->success($batch, 201);
    }

    private function presignPart(
        string $boardTable,
        string $ownerKey,
        SessionStore $store,
        ControlClient $client,
    ): never {
        $this->requireMethod('POST');
        $uploadId = $this->uploadId();
        $this->owned($store, $uploadId, $ownerKey, $boardTable);
        $body = $this->jsonBody();
        $partNumber = filter_var($body['part_number'] ?? null, FILTER_VALIDATE_INT);
        $contentLength = filter_var($body['content_length'] ?? null, FILTER_VALIDATE_INT);
        if (! is_int($partNumber) || ! is_int($contentLength)) {
            throw new UnexpectedValueException('invalid multipart part input');
        }
        $response = $this->batchValidator->validatePresignedPart(
            $client->presignPart($uploadId, $partNumber, $contentLength),
            $uploadId,
            $partNumber,
        );

        $this->success($response);
    }

    private function completeMultipart(
        string $boardTable,
        string $ownerKey,
        SessionStore $store,
        ControlClient $client,
    ): never {
        $this->requireMethod('POST');
        $uploadId = $this->uploadId();
        $this->owned($store, $uploadId, $ownerKey, $boardTable);
        $parts = $this->completedParts($this->jsonBody()['parts'] ?? null);
        $client->completeMultipart($uploadId, $parts);
        $store->markState($uploadId, 'quarantined');

        $this->success(null);
    }

    private function abortMultipart(
        string $boardTable,
        string $ownerKey,
        SessionStore $store,
        ControlClient $client,
    ): never {
        $this->requireMethod('DELETE');
        $uploadId = $this->uploadId();
        $this->owned($store, $uploadId, $ownerKey, $boardTable);
        $client->abortMultipart($uploadId);
        $store->markState($uploadId, 'aborted');

        $this->success(null);
    }

    private function confirmSingle(
        string $boardTable,
        string $ownerKey,
        SessionStore $store,
        ControlClient $client,
    ): never {
        $this->requireMethod('POST');
        $uploadId = $this->uploadId();
        $this->owned($store, $uploadId, $ownerKey, $boardTable);
        $client->confirmSingle($uploadId);
        $store->markState($uploadId, 'quarantined');

        $this->success(null);
    }

    private function status(
        string $boardTable,
        string $ownerKey,
        SessionStore $store,
        ControlClient $client,
    ): never {
        $this->requireMethod('GET');
        $uploadId = $this->uploadId();
        $this->owned($store, $uploadId, $ownerKey, $boardTable);
        $status = $client->status($uploadId);
        $this->assertStatusScope($status, $uploadId);
        if (is_string($status['state'] ?? null)) {
            $store->markState($uploadId, $status['state']);
        }

        $this->success($status);
    }

    private function prepare(
        string $boardTable,
        string $ownerKey,
        SessionStore $store,
        ControlClient $client,
    ): never {
        $this->requireMethod('POST');
        $uploadId = $this->uploadId();
        $session = $this->owned($store, $uploadId, $ownerKey, $boardTable);
        $asset = $this->readyValidator->validate($client->status($uploadId), $session);
        $store->markReady($uploadId, $asset);

        $this->success([
            'id' => (int) $asset['attachment_order'],
            'hash' => substr(hash('sha256', $uploadId), 0, 12),
            'original_filename' => $asset['original_filename'],
            'stored_filename' => $asset['stored_filename'],
            'mime_type' => $asset['mime_type'],
            'size' => $asset['size'],
            'url' => '',
            'preview_url' => null,
            'order' => $asset['attachment_order'],
        ]);
    }

    private function delete(
        string $boardTable,
        string $ownerKey,
        SessionStore $store,
        ControlClient $client,
    ): never {
        $this->requireMethod('DELETE');
        $uploadId = $this->uploadId();
        $this->owned($store, $uploadId, $ownerKey, $boardTable);
        $client->deleteUpload($uploadId);
        $store->completeDeletionRequest($uploadId);

        $this->success(null);
    }

    /** @return array<string, mixed> */
    private function owned(SessionStore $store, string $uploadId, string $ownerKey, string $boardTable): array
    {
        $session = $store->findOwned($uploadId, $ownerKey, $boardTable);
        if ($session === null) {
            throw new HttpFailure(404, 'UPLOAD_NOT_FOUND', '업로드 세션을 찾을 수 없습니다.');
        }

        return $session;
    }

    /** @return list<array{part_number:int,etag:string}> */
    private function completedParts(mixed $parts): array
    {
        if (! is_array($parts) || ! array_is_list($parts) || count($parts) < 1 || count($parts) > 10_000) {
            throw new UnexpectedValueException('invalid completed part list');
        }
        $validated = [];
        $previous = 0;
        foreach ($parts as $part) {
            $number = is_array($part) ? filter_var($part['part_number'] ?? null, FILTER_VALIDATE_INT) : false;
            $etag = is_array($part) ? ($part['etag'] ?? null) : null;
            if (! is_int($number)
                || $number !== $previous + 1
                || $number > 10_000
                || ! is_string($etag)
                || strlen($etag) < 1
                || strlen($etag) > 1024
                || ! preg_match('/^[\x21-\x7e]+$/', $etag)
            ) {
                throw new UnexpectedValueException('invalid completed part');
            }
            $validated[] = ['part_number' => $number, 'etag' => $etag];
            $previous = $number;
        }

        return $validated;
    }

    /** @param array<string, mixed> $status */
    private function assertStatusScope(array $status, string $uploadId): void
    {
        $states = ['created', 'uploaded', 'quarantined', 'processing', 'ready', 'rejected', 'failed', 'deleted'];
        if (($status['upload_id'] ?? null) !== $uploadId
            || ! in_array($status['state'] ?? null, $states, true)
            || ! is_bool($status['deletion_pending'] ?? null)
            || ! is_array($status['derivatives'] ?? null)
            || ! array_is_list($status['derivatives'])
            || count($status['derivatives']) > 2
        ) {
            throw new UnexpectedValueException('invalid upload status response');
        }
    }

    /** @return array<string, mixed> */
    private function jsonBody(): array
    {
        $length = filter_var($_SERVER['CONTENT_LENGTH'] ?? 0, FILTER_VALIDATE_INT);
        if (is_int($length) && $length > self::MAX_JSON_BYTES) {
            throw new HttpFailure(413, 'CONTROL_BODY_TOO_LARGE', '제어 요청 본문이 너무 큽니다.');
        }
        $body = file_get_contents('php://input', false, null, 0, self::MAX_JSON_BYTES + 1);
        if (! is_string($body) || strlen($body) > self::MAX_JSON_BYTES) {
            throw new HttpFailure(413, 'CONTROL_BODY_TOO_LARGE', '제어 요청 본문이 너무 큽니다.');
        }
        try {
            $decoded = json_decode($body, true, 32, JSON_THROW_ON_ERROR);
        } catch (JsonException) {
            throw new HttpFailure(400, 'INVALID_JSON', 'JSON 요청 본문이 올바르지 않습니다.');
        }
        if (! is_array($decoded) || array_is_list($decoded)) {
            throw new HttpFailure(400, 'INVALID_JSON_OBJECT', 'JSON 객체 요청이 필요합니다.');
        }

        return $decoded;
    }

    private function uploadId(): string
    {
        $uploadId = is_string($_GET['upload_id'] ?? null) ? strtolower($_GET['upload_id']) : '';
        if (! preg_match('/^[a-f0-9]{8}-[a-f0-9]{4}-[1-8][a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$/', $uploadId)) {
            throw new HttpFailure(404, 'UPLOAD_NOT_FOUND', '업로드 세션을 찾을 수 없습니다.');
        }

        return $uploadId;
    }

    private function requireMethod(string $method): void
    {
        if ($this->method() !== $method) {
            header('Allow: '.$method);
            throw new HttpFailure(405, 'METHOD_NOT_ALLOWED', '허용되지 않은 요청 방식입니다.');
        }
    }

    private function method(): string
    {
        return strtoupper((string) ($_SERVER['REQUEST_METHOD'] ?? 'GET'));
    }

    private function success(mixed $data, int $status = 200): never
    {
        $this->respond($status, ['success' => true, 'data' => $data]);
    }

    private function failure(int $status, string $code, string $message, ?string $requestId = null): never
    {
        $payload = ['success' => false, 'code' => $code, 'message' => $message];
        if ($requestId !== null) {
            $payload['request_id'] = $requestId;
        }
        $this->respond($status, $payload);
    }

    /** @param array<string, mixed> $payload */
    private function respond(int $status, array $payload): never
    {
        http_response_code($status);
        header('Content-Type: application/json; charset=utf-8');
        header('Cache-Control: private, no-store');
        header('X-Content-Type-Options: nosniff');
        header("Content-Security-Policy: default-src 'none'; frame-ancestors 'none'");
        header('Referrer-Policy: no-referrer');
        echo json_encode($payload, JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE | JSON_INVALID_UTF8_SUBSTITUTE);
        exit;
    }
}
