<?php

declare(strict_types=1);

use Illuminate\Support\Facades\Route;
use Modules\Jiwonpapa\G7mediabooster\Http\Controllers\Admin\SettingsController;
use Modules\Jiwonpapa\G7mediabooster\Http\Controllers\User\UploadController;

Route::prefix('admin')
    ->middleware(['auth:sanctum', 'admin', 'throttle:120,1'])
    ->group(function (): void {
        Route::get('settings', [SettingsController::class, 'show'])
            ->middleware('permission:admin,jiwonpapa-g7mediabooster.settings.read')
            ->name('admin.settings.show');
        Route::get('capabilities', [SettingsController::class, 'capabilities'])
            ->middleware('permission:admin,jiwonpapa-g7mediabooster.settings.read')
            ->name('admin.capabilities.show');
        Route::put('settings', [SettingsController::class, 'update'])
            ->middleware('permission:admin,jiwonpapa-g7mediabooster.settings.update')
            ->name('admin.settings.update');
    });

Route::prefix('boards/{slug}/uploads')
    ->middleware([
        'auth:sanctum',
        'throttle:1200,1',
        'permission:user,sirsoft-board.{slug}.attachments.upload',
    ])
    ->where(['slug' => '[A-Za-z0-9_-]+'])
    ->name('uploads.')
    ->group(function (): void {
        Route::get('configuration', [UploadController::class, 'configuration'])->name('configuration');
        Route::post('batches', [UploadController::class, 'create'])->name('batches.create');
        Route::post('{uploadId}/parts/{partNumber}/presign', [UploadController::class, 'presignPart'])
            ->whereUuid('uploadId')
            ->whereNumber('partNumber')
            ->name('parts.presign');
        Route::post('{uploadId}/multipart/complete', [UploadController::class, 'completeMultipart'])
            ->whereUuid('uploadId')
            ->name('multipart.complete');
        Route::delete('{uploadId}/multipart', [UploadController::class, 'abortMultipart'])
            ->whereUuid('uploadId')
            ->name('multipart.abort');
        Route::post('{uploadId}/complete', [UploadController::class, 'confirmSingle'])
            ->whereUuid('uploadId')
            ->name('single.complete');
        Route::get('{uploadId}', [UploadController::class, 'status'])
            ->whereUuid('uploadId')
            ->name('status');
        Route::delete('{uploadId}', [UploadController::class, 'delete'])
            ->whereUuid('uploadId')
            ->name('delete');
    });
