<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Services;

use Illuminate\Support\Facades\DB;
use Illuminate\Support\Facades\Schema;
use LogicException;

/** Keep native media URLs readable until remote attachments have been migrated or deleted. */
final class BridgeRemovalGuard
{
    public function assertSafe(): void
    {
        if (Schema::hasTable('g7mb_upload_sessions') && DB::table('g7mb_upload_sessions')
            ->whereNotNull('materialized_at')->whereNull('deletion_requested_at')->exists()) {
            throw new LogicException('G7MB_REMOTE_ATTACHMENTS_EXIST: disable uploads in settings; keep the delivery bridge active until media migration/deletion is complete.');
        }
    }
}
