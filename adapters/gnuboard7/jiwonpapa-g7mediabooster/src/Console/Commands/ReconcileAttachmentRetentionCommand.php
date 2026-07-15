<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Console\Commands;

use Illuminate\Console\Command;
use Modules\Jiwonpapa\G7mediabooster\Exceptions\MediaBoosterUpstreamException;
use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentRetentionDecision;
use Modules\Jiwonpapa\G7mediabooster\Services\AttachmentRetentionService;
use Modules\Jiwonpapa\G7mediabooster\Services\MediaBoosterClient;
use Throwable;

final class ReconcileAttachmentRetentionCommand extends Command
{
    protected $signature = 'g7mediabooster:reconcile-attachment-retention {--limit=50 : Maximum claims per run}';

    protected $description = '보존기간이 끝난 G7 미디어 첨부의 원격 삭제를 안전하게 예약합니다.';

    public function __construct(
        private readonly AttachmentRetentionService $retention,
        private readonly MediaBoosterClient $client,
    ) {
        parent::__construct();
    }

    public function handle(): int
    {
        $limit = filter_var($this->option('limit'), FILTER_VALIDATE_INT);
        if (! is_int($limit) || $limit < 1 || $limit > 100) {
            $this->error('limit must be between 1 and 100');

            return Command::INVALID;
        }

        $claimed = $this->retention->claimDue($limit);
        $requested = 0;
        $cancelled = 0;
        $failed = 0;
        foreach ($claimed as $session) {
            $uploadId = is_string($session['upload_id'] ?? null) ? strtolower($session['upload_id']) : '';
            if ($uploadId === '') {
                continue;
            }
            $decision = $this->retention->beginClaim($session);
            if ($decision === AttachmentRetentionDecision::CANCEL) {
                $this->retention->cancelClaim($uploadId);
                $cancelled++;
                continue;
            }
            if ($decision === AttachmentRetentionDecision::BLOCK) {
                $failed++;
                continue;
            }

            try {
                $this->client->deleteUpload($uploadId);
                $this->retention->completeClaim($uploadId);
                $requested++;
            } catch (MediaBoosterUpstreamException $error) {
                if ($error->httpStatus === 404) {
                    $this->retention->completeClaim($uploadId);
                    $requested++;
                    continue;
                }
                $this->retention->failClaim(
                    $uploadId,
                    $error->errorCode,
                    keepInFlight: $error->errorCode === 'UPSTREAM_UNAVAILABLE',
                );
                $failed++;
            } catch (Throwable) {
                $this->retention->failClaim($uploadId, 'RETENTION_REQUEST_FAILED', keepInFlight: true);
                $failed++;
            }
        }

        $this->info(sprintf(
            'attachment retention: claimed=%d requested=%d cancelled=%d failed=%d',
            count($claimed),
            $requested,
            $cancelled,
            $failed,
        ));

        return $failed === 0 ? Command::SUCCESS : Command::FAILURE;
    }
}
