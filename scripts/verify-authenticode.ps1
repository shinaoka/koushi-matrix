param(
    [Parameter(Mandatory = $true)]
    [string] $Path,

    [Parameter(Mandatory = $false)]
    [string] $ExpectedProduct = "Koushi"
)

$ErrorActionPreference = "Stop"

if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
    throw "File not found: $Path"
}

$item = Get-Item -LiteralPath $Path
if ($item.Length -le 0) {
    throw "File is empty: $Path"
}

$signature = Get-AuthenticodeSignature -FilePath $item.FullName
Write-Host "Signature status: $($signature.Status)"
Write-Host "Signature type: $($signature.SignatureType)"

if (-not $signature.SignerCertificate) {
    throw "No signer certificate was returned for $Path"
}

if ($signature.Status -ne [System.Management.Automation.SignatureStatus]::Valid) {
    throw "Authenticode signature is not valid: $($signature.Status) - $($signature.StatusMessage)"
}

if ([string]$signature.SignatureType -ne "Authenticode") {
    throw "Unexpected signature type: $($signature.SignatureType)"
}

if ([string]::IsNullOrWhiteSpace($ExpectedProduct)) {
    throw "ExpectedProduct must not be blank"
}

Write-Host "Signer subject: $($signature.SignerCertificate.Subject)"
Write-Host "Signer issuer: $($signature.SignerCertificate.Issuer)"
if ($signature.TimeStamperCertificate) {
    Write-Host "Timestamp subject: $($signature.TimeStamperCertificate.Subject)"
}

$hash = (Get-FileHash -Algorithm SHA256 -LiteralPath $item.FullName).Hash.ToLower()
Write-Host "SHA-256: $hash"
Write-Host "Authenticode verification passed."
