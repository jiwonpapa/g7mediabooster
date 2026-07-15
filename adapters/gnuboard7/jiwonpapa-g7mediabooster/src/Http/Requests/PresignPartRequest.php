<?php

declare(strict_types=1);

namespace Modules\Jiwonpapa\G7mediabooster\Http\Requests;

use Illuminate\Foundation\Http\FormRequest;

final class PresignPartRequest extends FormRequest
{
    public function authorize(): bool
    {
        return true;
    }

    /** @return array<string, mixed> */
    public function rules(): array
    {
        return ['content_length' => ['required', 'integer', 'min:1', 'max:5368709120']];
    }
}
