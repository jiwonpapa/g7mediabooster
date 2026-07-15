<?php

declare(strict_types=1);

use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\Schema;

return new class extends Migration
{
    public function up(): void
    {
        Schema::create('g7mb_upload_sessions', function (Blueprint $table): void {
            $table->uuid('upload_id')->primary();
            $table->uuid('batch_id')->index();
            $table->foreignId('user_id')->constrained('users')->cascadeOnDelete();
            $table->string('board_slug', 100);
            $table->string('client_ref', 128);
            $table->string('transfer_method', 20);
            $table->unsignedBigInteger('expected_size_bytes');
            $table->string('state', 32)->default('created');
            $table->timestamp('ownership_expires_at')->index();
            $table->timestamps();

            $table->index(
                ['user_id', 'board_slug', 'created_at'],
                'g7mb_upload_sessions_user_board_created_index',
            );
            $table->unique(
                ['batch_id', 'client_ref'],
                'g7mb_upload_sessions_batch_client_ref_unique',
            );
        });
    }

    public function down(): void
    {
        Schema::dropIfExists('g7mb_upload_sessions');
    }
};
