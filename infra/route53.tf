# The hosted zone for sbvh.nl already exists in the SBVH personal
# account. We reference it; we don't manage it. Records created by
# this Terraform stack go alongside whatever else lives in the zone.

# A-record pointing at the Lightsail relay's public IP. CloudFront's
# relay distribution uses this name as its origin, so when the box's
# IP changes (stop/start, replacement) this record is what gets
# updated. Imported from a record originally created via
# `aws route53 change-resource-record-sets`.
#
# When the static IP question is revisited, the `records` value
# changes to the static IP value and stays put.
resource "aws_route53_record" "origin_relay" {
  zone_id = data.aws_route53_zone.root.zone_id
  name    = "origin-relay.sbvh.nl"
  type    = "A"
  ttl     = 60
  records = [aws_lightsail_instance.relay.public_ip_address]
}

data "aws_route53_zone" "root" {
  name         = "${var.root_domain}."
  private_zone = false
}

# A-alias for the game (CloudFront distribution).
resource "aws_route53_record" "game" {
  zone_id = data.aws_route53_zone.root.zone_id
  name    = local.game_fqdn
  type    = "A"

  alias {
    name                   = aws_cloudfront_distribution.static.domain_name
    zone_id                = aws_cloudfront_distribution.static.hosted_zone_id
    evaluate_target_health = false
  }
}

# A-alias for the relay (CloudFront distribution).
resource "aws_route53_record" "relay" {
  zone_id = data.aws_route53_zone.root.zone_id
  name    = local.relay_fqdn
  type    = "A"

  alias {
    name                   = aws_cloudfront_distribution.relay.domain_name
    zone_id                = aws_cloudfront_distribution.relay.hosted_zone_id
    evaluate_target_health = false
  }
}

# ACM DNS-validation records — created from the for_each over the
# domain_validation_options that the certs surface. ACM uses DNS-01;
# Route 53 owns the zone, so validation completes within a minute or
# two of records appearing.

resource "aws_route53_record" "game_cert_validation" {
  for_each = {
    for dvo in aws_acm_certificate.game.domain_validation_options : dvo.domain_name => {
      name   = dvo.resource_record_name
      record = dvo.resource_record_value
      type   = dvo.resource_record_type
    }
  }

  zone_id         = data.aws_route53_zone.root.zone_id
  name            = each.value.name
  type            = each.value.type
  records         = [each.value.record]
  ttl             = 60
  allow_overwrite = true
}

resource "aws_route53_record" "relay_cert_validation" {
  for_each = {
    for dvo in aws_acm_certificate.relay.domain_validation_options : dvo.domain_name => {
      name   = dvo.resource_record_name
      record = dvo.resource_record_value
      type   = dvo.resource_record_type
    }
  }

  zone_id         = data.aws_route53_zone.root.zone_id
  name            = each.value.name
  type            = each.value.type
  records         = [each.value.record]
  ttl             = 60
  allow_overwrite = true
}
