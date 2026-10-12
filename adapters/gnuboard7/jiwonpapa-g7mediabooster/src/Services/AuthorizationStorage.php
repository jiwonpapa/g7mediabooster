<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use App\Contracts\Extension\StorageInterface;
use LogicException;
use Symfony\Component\HttpFoundation\StreamedResponse;

/** Metadata-only storage: native attachment authorization never reads media bytes. */
final class AuthorizationStorage implements StorageInterface
{
    public function __construct(private readonly string $authorizedPath) {}

    public function exists(string $category, string $path): bool
    {
        return $category === 'attachments' && hash_equals($this->authorizedPath, $path);
    }

    public function response(string $category, string $path, string $filename, array $headers = []): ?StreamedResponse
    {
        if (! $this->exists($category, $path)) {
            return null;
        }

        return new StreamedResponse(static function (): void {
            throw new LogicException('G7MB_AUTHORIZATION_RESPONSE_MUST_NOT_BE_SENT');
        }, 200, $headers);
    }

    public function download(string $category, string $path, string $filename, array $headers = []): ?StreamedResponse
    {
        return $this->response($category, $path, $filename, $headers);
    }

    public function getBasePath(string $category): string
    {
        return '/g7mb-authorization-only';
    }

    public function getDisk(): string
    {
        return 'g7mediabooster';
    }

    public function withDisk(string $disk): static
    {
        throw new LogicException('G7MB_AUTHORIZATION_IO_FORBIDDEN');
    }

    public function put(string $category, string $path, mixed $content): bool
    {
        throw new LogicException('G7MB_AUTHORIZATION_IO_FORBIDDEN');
    }

    public function get(string $category, string $path): ?string
    {
        throw new LogicException('G7MB_AUTHORIZATION_IO_FORBIDDEN');
    }

    public function delete(string $category, string $path): bool
    {
        throw new LogicException('G7MB_AUTHORIZATION_IO_FORBIDDEN');
    }

    public function deleteAll(string $category): bool
    {
        throw new LogicException('G7MB_AUTHORIZATION_IO_FORBIDDEN');
    }

    public function deleteDirectory(string $category, string $directory = ''): bool
    {
        throw new LogicException('G7MB_AUTHORIZATION_IO_FORBIDDEN');
    }

    public function files(string $category, string $directory = ''): array
    {
        throw new LogicException('G7MB_AUTHORIZATION_IO_FORBIDDEN');
    }

    public function url(string $category, string $path): ?string
    {
        throw new LogicException('G7MB_AUTHORIZATION_IO_FORBIDDEN');
    }
}
