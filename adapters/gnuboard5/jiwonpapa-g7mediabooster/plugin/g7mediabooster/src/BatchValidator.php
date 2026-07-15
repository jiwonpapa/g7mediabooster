<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use UnexpectedValueException;

final class BatchValidator
{
    private const IMAGE_TYPES = [
        'application/octet-stream',
        'image/avif',
        'image/gif',
        'image/heic',
        'image/heif',
        'image/jpeg',
        'image/png',
        'image/webp',
    ];

    /**
     * @param mixed $files
     * @return list<array{client_ref:string,original_filename:string,declared_kind:string,content_length:int,content_type_hint:string}>
     */
    public function validateRequest(mixed $files, int $maxFiles, int $maxBytes): array
    {
        if (! is_array($files)
            || ! array_is_list($files)
            || count($files) < 1
            || count($files) > min(100, max(1, $maxFiles))
            || $maxBytes < 1
        ) {
            throw new UnexpectedValueException('upload batch is outside the board limit');
        }

        $validated = [];
        $clientRefs = [];
        foreach ($files as $file) {
            if (! is_array($file) || array_is_list($file)) {
                throw new UnexpectedValueException('invalid upload file intent');
            }
            $clientRef = $file['client_ref'] ?? null;
            $filename = $file['original_filename'] ?? null;
            $kind = $file['declared_kind'] ?? null;
            $length = filter_var($file['content_length'] ?? null, FILTER_VALIDATE_INT);
            $contentType = $file['content_type_hint'] ?? null;
            if (! is_string($clientRef)
                || ! preg_match('/^[A-Za-z0-9_-]{1,64}$/', $clientRef)
                || isset($clientRefs[$clientRef])
                || ! is_string($filename)
                || $filename === ''
                || mb_strlen($filename, 'UTF-8') > 255
                || preg_match('#[\x00-\x1F\x7F/\\\\]#u', $filename)
                || ! in_array($kind, ['image', 'video'], true)
                || ! is_int($length)
                || $length < 1
                || $length > $maxBytes
                || ! is_string($contentType)
                || strlen($contentType) < 1
                || strlen($contentType) > 255
                || ! preg_match('/^[\x21-\x7e]+$/', $contentType)
            ) {
                throw new UnexpectedValueException('invalid upload file intent');
            }
            $allowedTypes = $kind === 'image' ? self::IMAGE_TYPES : ['application/octet-stream', 'video/mp4'];
            if (! in_array(strtolower($contentType), $allowedTypes, true)) {
                throw new UnexpectedValueException('media type is not release-supported');
            }

            $clientRefs[$clientRef] = true;
            $validated[] = [
                'client_ref' => $clientRef,
                'original_filename' => $filename,
                'declared_kind' => $kind,
                'content_length' => $length,
                'content_type_hint' => strtolower($contentType),
            ];
        }

        return $validated;
    }

    /**
     * @param array<string, mixed> $response
     * @param list<array{client_ref:string,original_filename:string,declared_kind:string,content_length:int,content_type_hint:string}> $files
     * @return array{batch_id:string,uploads:list<array<string,mixed>>}
     */
    public function validateResponse(array $response, array $files): array
    {
        $batchId = $this->uuid($response['batch_id'] ?? null);
        $uploads = $response['uploads'] ?? null;
        if (! is_array($uploads) || ! array_is_list($uploads) || count($uploads) !== count($files)) {
            throw new UnexpectedValueException('batch response count mismatch');
        }

        $requestRefs = array_column($files, 'client_ref');
        $seenUploads = [];
        foreach ($uploads as $index => $upload) {
            if (! is_array($upload) || array_is_list($upload)) {
                throw new UnexpectedValueException('invalid upload instruction');
            }
            $clientRef = $upload['client_ref'] ?? null;
            $uploadId = $this->uuid($upload['upload_id'] ?? null);
            $method = $upload['method'] ?? null;
            $headers = $upload['required_headers'] ?? null;
            $expiresAt = $upload['expires_at'] ?? null;
            if (! is_string($clientRef)
                || $clientRef !== $requestRefs[$index]
                || isset($seenUploads[$uploadId])
                || ! in_array($method, ['single_put', 'multipart'], true)
                || ! is_array($headers)
                || ($headers !== [] && array_is_list($headers))
                || ! is_string($expiresAt)
                || strtotime($expiresAt) === false
            ) {
                throw new UnexpectedValueException('invalid upload instruction');
            }
            foreach ($headers as $name => $value) {
                if (! is_string($name)
                    || ! preg_match('/^[A-Za-z0-9-]{1,128}$/', $name)
                    || ! is_string($value)
                    || strlen($value) > 4096
                    || preg_match('/[\r\n]/', $value)
                ) {
                    throw new UnexpectedValueException('invalid signed upload header');
                }
            }
            if ($method === 'single_put') {
                if (! $this->httpsOrLoopbackUrl($upload['upload_url'] ?? null)
                    || ($upload['part_size_bytes'] ?? null) !== null
                ) {
                    throw new UnexpectedValueException('invalid single upload instruction');
                }
            } else {
                $partSize = filter_var($upload['part_size_bytes'] ?? null, FILTER_VALIDATE_INT);
                if (($upload['upload_url'] ?? null) !== null
                    || ! is_int($partSize)
                    || $partSize < 5 * 1024 * 1024
                ) {
                    throw new UnexpectedValueException('invalid multipart upload instruction');
                }
            }
            $uploads[$index]['upload_id'] = $uploadId;
            $seenUploads[$uploadId] = true;
        }

        return ['batch_id' => $batchId, 'uploads' => $uploads];
    }

    /** @param array<string, mixed> $response @return array<string, mixed> */
    public function validatePresignedPart(array $response, string $uploadId, int $partNumber): array
    {
        // The control API binds the upload identifier in the request path; its
        // response intentionally contains only the signed part instruction.
        $this->uuid($uploadId);
        if (($response['part_number'] ?? null) !== $partNumber
            || ! $this->httpsOrLoopbackUrl($response['upload_url'] ?? null)
            || ! is_array($response['required_headers'] ?? null)
            || array_is_list($response['required_headers'])
            || ! is_string($response['expires_at'] ?? null)
            || strtotime($response['expires_at']) === false
        ) {
            throw new UnexpectedValueException('invalid presigned part response');
        }
        foreach ($response['required_headers'] as $name => $value) {
            if (! is_string($name)
                || ! preg_match('/^[A-Za-z0-9-]{1,128}$/', $name)
                || ! is_string($value)
                || strlen($value) > 4096
                || preg_match('/[\r\n]/', $value)
            ) {
                throw new UnexpectedValueException('invalid signed part header');
            }
        }

        return $response;
    }

    private function uuid(mixed $value): string
    {
        if (! is_string($value) || ! preg_match(
            '/^[a-fA-F0-9]{8}-[a-fA-F0-9]{4}-[1-8][a-fA-F0-9]{3}-[89abAB][a-fA-F0-9]{3}-[a-fA-F0-9]{12}$/',
            $value,
        )) {
            throw new UnexpectedValueException('invalid upload identifier');
        }

        return strtolower($value);
    }

    private function httpsOrLoopbackUrl(mixed $value): bool
    {
        if (! is_string($value) || strlen($value) > 8192) {
            return false;
        }
        $parts = parse_url($value);
        $host = strtolower((string) ($parts['host'] ?? ''));
        $scheme = strtolower((string) ($parts['scheme'] ?? ''));

        return ($scheme === 'https' || ($scheme === 'http' && in_array($host, ['127.0.0.1', '::1', 'localhost'], true)))
            && $host !== ''
            && ! isset($parts['user'])
            && ! isset($parts['pass'])
            && ! isset($parts['fragment']);
    }
}
