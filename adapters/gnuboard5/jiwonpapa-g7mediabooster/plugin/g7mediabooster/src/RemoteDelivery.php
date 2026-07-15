<?php

declare(strict_types=1);

namespace Jiwonpapa\G7MediaBooster\Gnuboard5;

use UnexpectedValueException;

final class RemoteDelivery
{
    public function __construct(
        private readonly GnuboardRuntime $runtime,
        private readonly DeliveryValidator $validator = new DeliveryValidator,
    ) {}

    /** @param array<string, mixed> $file */
    public function redirect(array $file, string $boardTable, int $writeId, string $variant): never
    {
        if (($file['bf_storage'] ?? null) !== 'g7mediabooster'
            || ($file['bo_table'] ?? null) !== $boardTable
            || (int) ($file['wr_id'] ?? 0) !== $writeId
            || ! in_array($variant, ['master', 'thumbnail'], true)
        ) {
            throw new UnexpectedValueException('invalid remote attachment scope');
        }
        $stored = (string) ($file['bf_file'] ?? '');
        if (! preg_match('/^g7mb-([a-f0-9-]{36})\.(?:jpg|mp4)$/', $stored, $matches)) {
            throw new UnexpectedValueException('invalid remote attachment key');
        }
        $uploadId = strtolower($matches[1]);
        $session = $this->runtime->store()->find($uploadId);
        if ($session === null
            || ($session['bo_table'] ?? null) !== $boardTable
            || (int) ($session['wr_id'] ?? 0) !== $writeId
            || (int) ($session['bf_no'] ?? -1) !== (int) ($file['bf_no'] ?? -2)
            || ($session['state'] ?? null) !== 'ready'
            || ($session['deletion_requested_at'] ?? null) !== null
        ) {
            throw new UnexpectedValueException('remote attachment mapping is not deliverable');
        }
        $url = $this->validator->validate(
            $this->runtime->client()->derivativeDelivery($uploadId, $variant),
            $uploadId,
            $variant,
        );

        http_response_code(302);
        header('Location: '.$url, true, 302);
        header('Cache-Control: private, no-store');
        header('Referrer-Policy: no-referrer');
        header('X-Content-Type-Options: nosniff');
        exit;
    }
}
