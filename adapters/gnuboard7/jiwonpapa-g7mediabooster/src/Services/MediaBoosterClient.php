<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use Illuminate\Http\Client\ConnectionException;
use Illuminate\Http\Client\Factory;
use Illuminate\Http\Client\Response;
use JsonException;
use LogicException;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Exceptions\MediaBoosterUpstreamException;

final class MediaBoosterClient
{
    public function __construct(
        private readonly MediaBoosterConfiguration $configuration,
        private readonly HmacRequestSigner $signer,
        private readonly Factory $http,
    ) {}

    /**
     * @param array<string, mixed> $payload
     * @return array<string, mixed>
     */
    public function createBatch(array $payload): array
    {
        return $this->expectObject($this->request('POST', '/v1/upload-batches', $payload));
    }

    /**
     * @return array<string, mixed>
     */
    public function presignPart(string $uploadId, int $partNumber, int $contentLength): array
    {
        $this->assertUploadId($uploadId);
        if ($partNumber < 1 || $partNumber > 10_000 || $contentLength < 1) {
            throw new LogicException('invalid multipart part request');
        }

        return $this->expectObject($this->request(
            'POST',
            "/v1/uploads/{$uploadId}/parts/{$partNumber}/presign",
            ['content_length' => $contentLength],
        ));
    }

    /**
     * @param array<int, array{part_number:int, etag:string}> $parts
     */
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

    public function deleteUpload(string $uploadId): void
    {
        $this->assertUploadId($uploadId);
        $this->request('DELETE', "/v1/uploads/{$uploadId}");
    }

    public function confirmSingle(string $uploadId): void
    {
        $this->assertUploadId($uploadId);
        $this->request('POST', "/v1/uploads/{$uploadId}/complete");
    }

    /**
     * @return array<string, mixed>
     */
    public function status(string $uploadId): array
    {
        $this->assertUploadId($uploadId);

        return $this->expectObject($this->request('GET', "/v1/uploads/{$uploadId}"));
    }

    /**
     * @return array<string, mixed>
     */
    public function capabilities(): array
    {
        return $this->expectObject($this->request('GET', '/v1/capabilities'));
    }

    /**
     * @param array<string, mixed> $payload
     * @return array<string, mixed>
     */
    public function publishSitePolicy(array $payload): array
    {
        return $this->expectObject($this->request('PUT', '/v1/site-policy', $payload));
    }

    /**
     * @return array<string, mixed>|null
     */
    public function activeSitePolicy(): ?array
    {
        try {
            return $this->expectObject($this->request('GET', '/v1/site-policy'));
        } catch (MediaBoosterUpstreamException $error) {
            if ($error->httpStatus === 404 && $error->errorCode === 'SITE_POLICY_NOT_FOUND') {
                return null;
            }

            throw $error;
        }
    }

    /**
     * @param array<string, mixed>|null $payload
     * @return array<string, mixed>|null
     */
    private function request(string $method, string $path, ?array $payload = null): ?array
    {
        if (! $this->configuration->enabled) {
            throw new MediaBoosterUpstreamException(503, 'MODULE_DISABLED', '미디어 부스터가 비활성화되어 있습니다.');
        }

        try {
            $body = $payload === null
                ? ''
                : json_encode($payload, JSON_THROW_ON_ERROR | JSON_UNESCAPED_SLASHES | JSON_UNESCAPED_UNICODE);
        } catch (JsonException) {
            throw new LogicException('request payload cannot be encoded');
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
            $response = $this->http
                ->withHeaders($headers)
                ->withBody($body, 'application/json')
                ->connectTimeout($this->configuration->connectTimeoutSeconds)
                ->timeout($this->configuration->timeoutSeconds)
                ->withoutRedirecting()
                ->send($method, $this->configuration->endpoint.$path);
        } catch (ConnectionException) {
            throw new MediaBoosterUpstreamException(
                503,
                'UPSTREAM_UNAVAILABLE',
                '미디어 처리 서버에 연결할 수 없습니다.',
            );
        }

        return $this->decodeResponse($response);
    }

    /**
     * @return array<string, mixed>|null
     */
    private function decodeResponse(Response $response): ?array
    {
        if ($response->successful()) {
            if ($response->status() === 204 || trim($response->body()) === '') {
                return null;
            }

            $decoded = $response->json();
            if (! is_array($decoded) || array_is_list($decoded)) {
                throw new MediaBoosterUpstreamException(502, 'INVALID_UPSTREAM_RESPONSE', '미디어 처리 서버 응답이 올바르지 않습니다.');
            }

            return $decoded;
        }

        $decoded = $response->json();
        $code = is_array($decoded) && is_string($decoded['code'] ?? null)
            ? substr($decoded['code'], 0, 80)
            : 'UPSTREAM_REQUEST_FAILED';
        $message = is_array($decoded) && is_string($decoded['message'] ?? null)
            ? substr($decoded['message'], 0, 240)
            : '미디어 처리 서버가 요청을 거부했습니다.';
        $requestId = is_array($decoded) && is_string($decoded['request_id'] ?? null)
            ? substr($decoded['request_id'], 0, 128)
            : null;
        $status = in_array($response->status(), [400, 401, 404, 409, 422, 429, 503], true)
            ? $response->status()
            : 502;

        throw new MediaBoosterUpstreamException($status, $code, $message, $requestId);
    }

    /**
     * @param array<string, mixed>|null $value
     * @return array<string, mixed>
     */
    private function expectObject(?array $value): array
    {
        if ($value === null) {
            throw new MediaBoosterUpstreamException(502, 'INVALID_UPSTREAM_RESPONSE', '미디어 처리 서버 응답이 비어 있습니다.');
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
