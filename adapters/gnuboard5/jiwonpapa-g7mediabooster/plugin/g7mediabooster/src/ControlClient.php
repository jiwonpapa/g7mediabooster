<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use JsonException;
use LogicException;
use Throwable;

final class ControlClient
{
    public function __construct(
        private readonly Configuration $configuration,
        private readonly HmacSigner $signer,
        private readonly Transport $transport,
    ) {}

    /** @param array<string, mixed> $payload @return array<string, mixed> */
    public function createBatch(array $payload): array
    {
        return $this->requiredObject($this->request('POST', '/v1/upload-batches', $payload));
    }

    /** @return array<string, mixed> */
    public function presignPart(string $uploadId, int $partNumber, int $contentLength): array
    {
        $this->assertUploadId($uploadId);
        if ($partNumber < 1 || $partNumber > 10_000 || $contentLength < 1) {
            throw new LogicException('invalid multipart part request');
        }

        return $this->requiredObject($this->request(
            'POST',
            "/v1/uploads/{$uploadId}/parts/{$partNumber}/presign",
            ['content_length' => $contentLength],
        ));
    }

    /** @param list<array{part_number:int, etag:string}> $parts */
    public function completeMultipart(string $uploadId, array $parts): void
    {
        $this->assertUploadId($uploadId);
        $this->request('POST', "/v1/uploads/{$uploadId}/multipart/complete", ['parts' => $parts]);
    }

    public function abortMultipart(string $uploadId): void
    {
        $this->assertUploadId($uploadId);
        $this->request('DELETE', "/v1/uploads/{$uploadId}/multipart");
    }

    public function confirmSingle(string $uploadId): void
    {
        $this->assertUploadId($uploadId);
        $this->request('POST', "/v1/uploads/{$uploadId}/complete");
    }

    public function deleteUpload(string $uploadId): void
    {
        $this->assertUploadId($uploadId);
        $this->request('DELETE', "/v1/uploads/{$uploadId}");
    }

    /** @return array<string, mixed> */
    public function status(string $uploadId): array
    {
        $this->assertUploadId($uploadId);

        return $this->requiredObject($this->request('GET', "/v1/uploads/{$uploadId}"));
    }

    /** @return array<string, mixed> */
    public function derivativeDelivery(string $uploadId, string $variant): array
    {
        $this->assertUploadId($uploadId);
        if (! in_array($variant, ['master', 'thumbnail'], true)) {
            throw new LogicException('invalid derivative variant');
        }

        return $this->requiredObject($this->request(
            'GET',
            "/v1/uploads/{$uploadId}/derivatives/{$variant}/delivery",
        ));
    }

    /** @param array<string, mixed>|null $payload @return array<string, mixed>|null */
    private function request(string $method, string $path, ?array $payload = null): ?array
    {
        if (! $this->configuration->enabled) {
            throw new UpstreamException(503, 'ADAPTER_DISABLED', '미디어 부스터가 비활성화되어 있습니다.');
        }
        try {
            $body = $payload === null ? '' : json_encode(
                $payload,
                JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE,
            );
        } catch (JsonException) {
            throw new LogicException('control payload cannot be encoded');
        }
        $headers = $this->signer->sign(
            $this->configuration->keyId,
            $this->configuration->hmacSecret,
            $method,
            $path,
            $body,
        );
        $headers['accept'] = 'application/json';
        $headers['content-type'] = 'application/json';

        try {
            $response = $this->transport->send(
                $method,
                $this->configuration->endpoint.$path,
                $headers,
                $body,
                $this->configuration->connectTimeoutSeconds,
                $this->configuration->timeoutSeconds,
            );
        } catch (Throwable) {
            throw new UpstreamException(503, 'UPSTREAM_UNAVAILABLE', '미디어 처리 서버에 연결할 수 없습니다.');
        }

        if ($response->status >= 200 && $response->status < 300) {
            if ($response->status === 204 || trim($response->body) === '') {
                return null;
            }

            return $this->decodeObject($response->body, 502, 'INVALID_UPSTREAM_RESPONSE');
        }

        $error = $this->decodeObject($response->body, 502, 'UPSTREAM_REQUEST_FAILED', false);
        $status = in_array($response->status, [400, 401, 404, 409, 422, 429, 503], true)
            ? $response->status
            : 502;
        $code = is_string($error['code'] ?? null) && preg_match('/^[A-Z0-9_]{1,80}$/', $error['code'])
            ? $error['code']
            : 'UPSTREAM_REQUEST_FAILED';
        $message = is_string($error['message'] ?? null)
            ? mb_substr($error['message'], 0, 240, 'UTF-8')
            : '미디어 처리 서버가 요청을 거부했습니다.';
        $requestId = is_string($error['request_id'] ?? null)
            ? substr($error['request_id'], 0, 128)
            : null;
        throw new UpstreamException($status, $code, $message, $requestId);
    }

    /** @return array<string, mixed> */
    private function decodeObject(string $body, int $status, string $code, bool $strict = true): array
    {
        try {
            $decoded = json_decode($body, true, 32, JSON_THROW_ON_ERROR);
        } catch (JsonException) {
            if (! $strict) {
                return [];
            }
            throw new UpstreamException($status, $code, '미디어 처리 서버 응답이 올바르지 않습니다.');
        }
        if (! is_array($decoded) || array_is_list($decoded)) {
            if (! $strict) {
                return [];
            }
            throw new UpstreamException($status, $code, '미디어 처리 서버 응답이 올바르지 않습니다.');
        }

        return $decoded;
    }

    /** @param array<string, mixed>|null $value @return array<string, mixed> */
    private function requiredObject(?array $value): array
    {
        if ($value === null) {
            throw new UpstreamException(502, 'INVALID_UPSTREAM_RESPONSE', '미디어 처리 서버 응답이 비어 있습니다.');
        }

        return $value;
    }

    private function assertUploadId(string $uploadId): void
    {
        if (! preg_match('/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/', $uploadId)) {
            throw new LogicException('invalid upload id');
        }
    }
}
