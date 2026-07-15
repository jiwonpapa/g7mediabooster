<?php

declare(strict_types=1);

use Illuminate\Database\Migrations\Migration;
use Illuminate\Database\Schema\Blueprint;
use Illuminate\Support\Facades\DB;
use Illuminate\Support\Facades\Schema;

return new class extends Migration
{
    public function up(): void
    {
        Schema::table('g7mb_upload_sessions', function (Blueprint $table): void {
            $table->dropForeign(['user_id']);
        });
        Schema::table('g7mb_upload_sessions', function (Blueprint $table): void {
            $table->unsignedBigInteger('user_id')->nullable()->change();
            $table->foreign('user_id')->references('id')->on('users')->nullOnDelete();
        });

        Schema::table('g7mb_upload_sessions', function (Blueprint $table): void {
            $table->string('original_filename', 255)->nullable()->after('client_ref');
            $table->string('declared_kind', 16)->nullable()->after('original_filename');
            $table->string('content_type_hint', 255)->nullable()->after('declared_kind');
            $table->unsignedSmallInteger('attachment_order')->nullable()->after('content_type_hint');
            $table->unsignedBigInteger('attachment_id')->nullable()->unique()->after('state');
            $table->timestamp('materialized_at')->nullable()->after('attachment_id');
        });
    }

    public function down(): void
    {
        Schema::table('g7mb_upload_sessions', function (Blueprint $table): void {
            $table->dropUnique(['attachment_id']);
            $table->dropColumn([
                'original_filename',
                'declared_kind',
                'content_type_hint',
                'attachment_order',
                'attachment_id',
                'materialized_at',
            ]);
        });

        DB::table('g7mb_upload_sessions')->whereNull('user_id')->delete();
        Schema::table('g7mb_upload_sessions', function (Blueprint $table): void {
            $table->dropForeign(['user_id']);
        });
        Schema::table('g7mb_upload_sessions', function (Blueprint $table): void {
            $table->unsignedBigInteger('user_id')->nullable(false)->change();
            $table->foreign('user_id')->references('id')->on('users')->cascadeOnDelete();
        });
    }
};
