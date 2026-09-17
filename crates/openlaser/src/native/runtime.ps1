# Verifies the embedded Microsoft bootstrapper before its one-time setup.
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
$form = New-Object System.Windows.Forms.Form
$form.Text = 'OpenLaser'
$form.Size = New-Object System.Drawing.Size(490, 190)
$form.StartPosition = 'CenterScreen'
$form.FormBorderStyle = 'FixedDialog'
$form.ControlBox = $false
$label = New-Object System.Windows.Forms.Label
$label.Location = New-Object System.Drawing.Point(24, 24)
$label.Size = New-Object System.Drawing.Size(430, 65)
$label.Text = "Preparing OpenLaser...`nSetting up the browser runtime. This needs internet only once."
$label.Font = New-Object System.Drawing.Font('Segoe UI', 11)
$form.Controls.Add($label)
$progress = New-Object System.Windows.Forms.ProgressBar
$progress.Location = New-Object System.Drawing.Point(24, 103)
$progress.Size = New-Object System.Drawing.Size(430, 12)
$progress.Style = 'Marquee'
$form.Controls.Add($progress)
$form.Show()
[System.Windows.Forms.Application]::DoEvents()
try {
    $signature = Get-AuthenticodeSignature -LiteralPath $env:OPENLASER_RUNTIME_INSTALLER
    if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Microsoft Corporation(,|$)') {
        throw 'Microsoft browser setup signature could not be verified.'
    }
    $setup = Start-Process -FilePath $env:OPENLASER_RUNTIME_INSTALLER -ArgumentList '/silent', '/install' -PassThru
    while (-not $setup.HasExited) {
        [System.Windows.Forms.Application]::DoEvents()
        Start-Sleep -Milliseconds 100
    }
    exit $setup.ExitCode
} finally {
    $form.Dispose()
}
