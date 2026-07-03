# Look up AWS-managed CloudFront policies by name instead of
# hardcoding magic UUIDs in cloudfront.tf. The IDs are stable but the
# names are documented and grep-able; using the data source is the
# pattern AWS recommends.
#
# Reference: https://docs.aws.amazon.com/AmazonCloudFront/latest/DeveloperGuide/using-managed-cache-policies.html

data "aws_cloudfront_cache_policy" "caching_optimized" {
  name = "Managed-CachingOptimized"
}

data "aws_cloudfront_cache_policy" "caching_disabled" {
  name = "Managed-CachingDisabled"
}

data "aws_cloudfront_origin_request_policy" "all_viewer" {
  name = "Managed-AllViewer"
}

# CORS on responses so module scripts (always fetched in CORS mode
# per the HTML spec) get `Access-Control-Allow-Origin` and browsers
# stop CORB-sanitising `window.onerror` details. See ERROR.md — errors
# are first-class primitives; sanitisation violates the axiom.
data "aws_cloudfront_response_headers_policy" "simple_cors" {
  name = "Managed-SimpleCORS"
}
