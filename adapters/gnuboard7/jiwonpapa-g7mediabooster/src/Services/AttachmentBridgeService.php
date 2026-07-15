<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use LogicException;
use Modules\Sirsoft\Board\Models\Attachment;
use Modules\Sirsoft\Board\Repositories\Contracts\AttachmentRepositoryInterface;
use Modules\Sirsoft\Board\Services\AttachmentService as BoardAttachmentService;
use ReflectionMethod;
use UnexpectedValueException;

final class AttachmentBridgeService
{
    public function __construct(
        private readonly UploadSessionStore $sessions,
        private readonly MediaBoosterClient $client,
        private readonly AttachmentMaterializationValidator $validator,
        private readonly AttachmentRepositoryInterface $attachments,
    ) {}

    public function materialize(string $uploadId, int $userId, string $boardSlug): Attachment
    {
        $this->assertSecureUpstreamContract();
        $session = $this->sessions->ownedSession($uploadId, $userId, $boardSlug);
        if ($session === null) {
            throw new UnexpectedValueException('upload session is not owned');
        }

        // HMAC network I/O is intentionally outside the database transaction.
        $status = $this->client->status($uploadId);
        $descriptor = $this->validator->validate($status, $session);
        $attachmentId = $this->sessions->materializeAttachment(
            $uploadId,
            $userId,
            $boardSlug,
            function () use ($boardSlug, $descriptor, $userId): int {
                $attachment = $this->attachments->create($boardSlug, [
                    ...$descriptor,
                    'created_by' => $userId,
                ]);
                $id = filter_var($attachment->getKey(), FILTER_VALIDATE_INT);

                return is_int($id) ? $id : 0;
            },
        );

        $attachment = Attachment::query()
            ->where('id', $attachmentId)
            ->where('board_id', 0)
            ->whereNull('post_id')
            ->where('created_by', $userId)
            ->first() ?? $this->attachments->findById($boardSlug, $attachmentId);
        if (! $attachment instanceof Attachment
            || (int) $attachment->created_by !== $userId
            || $attachment->disk !== 'g7mediabooster'
            || strtolower((string) $attachment->path) !== strtolower($uploadId)
        ) {
            throw new UnexpectedValueException('materialized attachment cannot be reloaded');
        }

        return $attachment;
    }

    public static function assertSecureUpstreamContract(): void
    {
        if (! method_exists(BoardAttachmentService::class, 'authorizeDelivery')) {
            throw new LogicException('sirsoft-board secure external attachment contract is unavailable');
        }

        if (! method_exists(AttachmentRepositoryInterface::class, 'findPostForAttachmentDelivery')) {
            throw new LogicException('sirsoft-board visibility-aware attachment delivery is unavailable');
        }

        $visibilityMethod = new ReflectionMethod(AttachmentRepositoryInterface::class, 'findPostForAttachmentDelivery');
        if ($visibilityMethod->getNumberOfParameters() !== 2) {
            throw new LogicException('sirsoft-board visibility-aware attachment delivery is incompatible');
        }

        $linkMethod = new ReflectionMethod(AttachmentRepositoryInterface::class, 'linkAttachmentsByIds');
        if ($linkMethod->getNumberOfParameters() !== 4) {
            throw new LogicException('sirsoft-board owner-bound attachment linking is unavailable');
        }
    }
}
