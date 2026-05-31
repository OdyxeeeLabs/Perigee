/**
 * Error handling utilities for parsing and formatting backend error responses
 */

export interface BackendErrorResponse {
  error: string;
  message: string;
  statusCode?: number;
}

export interface FormattedError {
  type: string;
  message: string;
  details?: string;
  statusCode: number;
  isNetworkError: boolean;
}

/**
 * Extract detailed error information from backend response
 */
export async function extractErrorDetails(response: Response): Promise<BackendErrorResponse> {
  try {
    const data = await response.json();
    return {
      error: data.error || 'UNKNOWN_ERROR',
      message: data.message || response.statusText || 'An error occurred',
      statusCode: response.status,
    };
  } catch {
    // If response body is not JSON, use status text
    return {
      error: getErrorType(response.status),
      message: response.statusText || 'An error occurred',
      statusCode: response.status,
    };
  }
}

/**
 * Map HTTP status codes to error types
 */
function getErrorType(status: number): string {
  switch (status) {
    case 400:
      return 'BAD_REQUEST';
    case 401:
      return 'UNAUTHORIZED';
    case 404:
      return 'NOT_FOUND';
    case 500:
      return 'INTERNAL_SERVER_ERROR';
    case 503:
      return 'SERVICE_UNAVAILABLE';
    default:
      return 'UNKNOWN_ERROR';
  }
}

/**
 * Format error for display
 */
export function formatError(error: unknown): FormattedError {
  if (error instanceof Response) {
    return {
      type: getErrorType(error.status),
      message: error.statusText || 'Network error',
      statusCode: error.status,
      isNetworkError: true,
    };
  }

  if (error instanceof TypeError) {
    return {
      type: 'NETWORK_ERROR',
      message: 'Failed to connect to backend. Please ensure the server is running.',
      details: error.message,
      statusCode: 0,
      isNetworkError: true,
    };
  }

  if (error instanceof Error) {
    // Check if it's a JSON parse error
    if (error.message.includes('JSON')) {
      return {
        type: 'PARSE_ERROR',
        message: 'Failed to parse response from backend',
        details: error.message,
        statusCode: 0,
        isNetworkError: false,
      };
    }

    return {
      type: 'ERROR',
      message: error.message || 'An unexpected error occurred',
      statusCode: 0,
      isNetworkError: false,
    };
  }

  return {
    type: 'UNKNOWN_ERROR',
    message: 'An unexpected error occurred',
    statusCode: 0,
    isNetworkError: false,
  };
}

/**
 * Create a user-friendly error message from BackendErrorResponse
 */
export function createUserFriendlyMessage(errorResponse: BackendErrorResponse): string {
  const errorMessages: Record<string, string> = {
    BAD_REQUEST: 'Invalid request. Please check your inputs and try again.',
    UNAUTHORIZED: 'You are not authorized to perform this action.',
    NOT_FOUND: 'The requested resource was not found.',
    INTERNAL_SERVER_ERROR: 'Server error. Please try again later.',
    SERVICE_UNAVAILABLE: 'The service is currently unavailable. Please try again later.',
  };

  return (
    errorMessages[errorResponse.error] ||
    errorResponse.message ||
    'An error occurred during analysis'
  );
}
