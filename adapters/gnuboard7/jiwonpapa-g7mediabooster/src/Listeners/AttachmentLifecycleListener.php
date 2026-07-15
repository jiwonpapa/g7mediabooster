<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Listeners;

use App\Contracts\Extension\HookListenerInterface;
use Modules\Jiwonpapa\G7mediabooster\Config\MediaBoosterConfiguration;
use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentRetentionService;

final class AttachmentLifecycleListener implements HookListenerInterface
{
    public function __construct(
        private readonly AttachmentRetentionService $retention,
        private readonly MediaBoosterConfiguration $configuration,
    ) {}

    /** @return array<string, array<string, mixed>> */
    public static function getSubscribedHooks(): array
    {
        return [
            'sirsoft-board.post.after_delete' => ['method' => 'postDeleted', 'priority' => 30, 'sync' => true],
            'sirsoft-board.post.before_restore' => ['method' => 'postRestoring', 'priority' => 1, 'sync' => true],
            'sirsoft-board.post.after_restore' => ['method' => 'postRestored', 'priority' => 30, 'sync' => true],
            'sirsoft-board.attachment.after_delete' => ['method' => 'attachmentDeleted', 'priority' => 30, 'sync' => true],
        ];
    }

    public function handle(...$args): void {}

    /** @param array<string, mixed> $options */
    public function postDeleted(mixed $post, string $boardSlug, array $options = []): void
    {
        $postId = $this->modelId($post);
        if ($postId > 0) {
            $this->retention->schedulePostDeletion($postId, $boardSlug, $this->configuration->attachmentRetentionDays);
        }
    }

    public function postRestored(mixed $post, string $boardSlug): void
    {
        $postId = $this->modelId($post);
        if ($postId > 0) {
            $this->retention->cancelRestoredPost($postId, $boardSlug);
        }
    }

    public function postRestoring(mixed $post, mixed $reason, string $boardSlug): void
    {
        $postId = $this->modelId($post);
        if ($postId > 0) {
            $this->retention->preparePostRestore($postId, $boardSlug);
        }
    }

    public function attachmentDeleted(mixed $attachment): void
    {
        $attachmentId = $this->modelId($attachment);
        $boardSlug = $this->modelBoardSlug($attachment);
        if ($attachmentId > 0 && $boardSlug !== null) {
            $this->retention->scheduleAttachmentDeletion(
                $attachmentId,
                $boardSlug,
                $this->configuration->attachmentRetentionDays,
            );
        }
    }

    private function modelId(mixed $model): int
    {
        if (is_object($model) && method_exists($model, 'getKey')) {
            return (int) $model->getKey();
        }

        return 0;
    }

    private function modelBoardSlug(mixed $model): ?string
    {
        if (! is_object($model)) {
            return null;
        }
        $board = $model->board ?? null;
        $slug = is_object($board) ? ($board->slug ?? null) : null;

        return is_string($slug) && preg_match('/^[A-Za-z0-9_-]+$/', $slug) === 1 ? $slug : null;
    }
}
