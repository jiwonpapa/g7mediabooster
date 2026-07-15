<?php

declare(strict_types=1);

use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\Schema;

return new class extends Migration
{
    public function up(): void
    {
        Schema::table('g7mb_upload_sessions', function (Blueprint $table): void {
            $table->timestamp('retention_delete_after')->nullable()->after('materialized_at');
            $table->string('retention_reason', 32)->nullable()->after('retention_delete_after');
            $table->unsignedSmallInteger('retention_attempts')->default(0)->after('retention_reason');
            $table->timestamp('retention_lease_until')->nullable()->after('retention_attempts');
            $table->timestamp('retention_request_started_at')->nullable()->after('retention_lease_until');
            $table->timestamp('deletion_requested_at')->nullable()->after('retention_request_started_at');
            $table->string('retention_last_error', 128)->nullable()->after('deletion_requested_at');
            $table->index(
                ['retention_delete_after', 'retention_lease_until', 'retention_attempts'],
                'g7mb_upload_sessions_retention_due_index',
            );
        });
    }

    public function down(): void
    {
        Schema::table('g7mb_upload_sessions', function (Blueprint $table): void {
            $table->dropIndex('g7mb_upload_sessions_retention_due_index');
            $table->dropColumn([
                'retention_delete_after',
                'retention_reason',
                'retention_attempts',
                'retention_lease_until',
                'retention_request_started_at',
                'deletion_requested_at',
                'retention_last_error',
            ]);
        });
    }
};
