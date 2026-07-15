<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Listeners;

use App\Contracts\Extension\HookListenerInterface;
use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentUrlResolver;

final class AttachmentUrlListener implements HookListenerInterface
{
    public function __construct(private readonly AttachmentUrlResolver $resolver) {}

    /** @return array<string, array<string, mixed>> */
    public static function getSubscribedHooks(): array
    {
        return [
            'sirsoft-board.attachment.filter_download_url' => [
                'method' => 'downloadUrl',
                'priority' => 10,
                'type' => 'filter',
                'sync' => true,
            ],
            'sirsoft-board.attachment.filter_preview_url' => [
                'method' => 'previewUrl',
                'priority' => 10,
                'type' => 'filter',
                'sync' => true,
            ],
        ];
    }

    public function handle(...$args): void {}

    public function downloadUrl(?string $url, mixed $attachment, ?string $boardSlug = null): ?string
    {
        return $this->resolver->resolve($url, $attachment, 'master', $boardSlug);
    }

    public function previewUrl(?string $url, mixed $attachment, ?string $boardSlug = null): ?string
    {
        return $this->resolver->resolve($url, $attachment, 'thumbnail', $boardSlug);
    }
}
