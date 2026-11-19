# Dumps the UI Automation tree (what Narrator and other screen readers read) of the top-level window
# of a process, as JSON: name, control type, ARIA role and properties, help text, keyboard focus,
# toggle and expand/collapse state. Run with Windows PowerShell 5.1 (powershell.exe).
param(
  [Parameter(Mandatory = $true)][int]$ProcessId,
  [Parameter(Mandatory = $true)][string]$Out,
  [int]$WaitSeconds = 25
)
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName UIAutomationClient
Add-Type -AssemblyName UIAutomationTypes
$AE = [System.Windows.Automation.AutomationElement]
$scopeChildren = [System.Windows.Automation.TreeScope]::Children
$scopeDescendants = [System.Windows.Automation.TreeScope]::Descendants

$window = $null
$deadline = (Get-Date).AddSeconds($WaitSeconds)
while (-not $window -and (Get-Date) -lt $deadline) {
  $condition = New-Object System.Windows.Automation.PropertyCondition($AE::ProcessIdProperty, $ProcessId)
  $window = $AE::RootElement.FindFirst($scopeChildren, $condition)
  if (-not $window) { Start-Sleep -Milliseconds 300 }
}
if (-not $window) { throw "No top-level window for process $ProcessId" }

$request = New-Object System.Windows.Automation.CacheRequest
# The ARIA identifiers are not static members on every .NET Framework: look them up by their UIA ids
# (30101 AriaRole, 30102 AriaProperties).
$ariaRoles = [System.Windows.Automation.AutomationProperty]::LookupById(30101)
$ariaProperties = [System.Windows.Automation.AutomationProperty]::LookupById(30102)
$wanted = @(
  $AE::NameProperty, $AE::ControlTypeProperty, $AE::LocalizedControlTypeProperty, $AE::HelpTextProperty,
  $AE::IsKeyboardFocusableProperty, $AE::HasKeyboardFocusProperty, $AE::IsOffscreenProperty,
  $AE::ClassNameProperty, $AE::BoundingRectangleProperty, $ariaRoles, $ariaProperties)
foreach ($property in $wanted) {
  if ($null -ne $property) { $request.Add([System.Windows.Automation.AutomationProperty]$property) }
}
$request.TreeScope = [System.Windows.Automation.TreeScope]::Element
$request.TreeFilter = [System.Windows.Automation.Automation]::ControlViewCondition

# Chromium builds its accessibility tree lazily, once a client asks: ask, wait, ask again.
$elements = @()
for ($attempt = 0; $attempt -lt 6; $attempt++) {
  $activation = $request.Activate()
  try { $elements = @($window.FindAll($scopeDescendants, [System.Windows.Automation.Automation]::ControlViewCondition)) }
  finally { $activation.Dispose() }
  if ($elements.Count -gt 40) { break }
  Start-Sleep -Milliseconds 1200
}

$rows = foreach ($element in $elements) {
  $toggle = $null; $expand = $null; $selected = $null
  # States come from the live patterns (the cached ones are not filled for Chromium's elements); only for
  # the control types that carry them, to keep the dump quick.
  if ($element.Cached.ControlType.ProgrammaticName -match 'Button$|CheckBox$|TabItem$|RadioButton$|MenuItem$') {
    $pattern = $null
    if ($element.TryGetCurrentPattern([System.Windows.Automation.TogglePattern]::Pattern, [ref]$pattern)) { $toggle = $pattern.Current.ToggleState.ToString() }
    $pattern = $null
    if ($element.TryGetCurrentPattern([System.Windows.Automation.ExpandCollapsePattern]::Pattern, [ref]$pattern)) { $expand = $pattern.Current.ExpandCollapseState.ToString() }
    $pattern = $null
    if ($element.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$pattern)) { $selected = $pattern.Current.IsSelected }
  }
  $rect = $element.Cached.BoundingRectangle
  [pscustomobject]@{
    type      = $element.Cached.ControlType.ProgrammaticName -replace '^ControlType\.', ''
    localized = $element.Cached.LocalizedControlType
    name      = $element.Cached.Name
    help      = $element.Cached.HelpText
    roles     = $(if ($null -ne $ariaRoles) { $element.GetCachedPropertyValue($ariaRoles) })
    props     = $(if ($null -ne $ariaProperties) { $element.GetCachedPropertyValue($ariaProperties) })
    focusable = $element.Cached.IsKeyboardFocusable
    focused   = $element.Cached.HasKeyboardFocus
    offscreen = $element.Cached.IsOffscreen
    toggle    = $toggle
    expand    = $expand
    selected  = $selected
    x         = if ($rect.IsEmpty) { $null } else { [int]$rect.X }
    y         = if ($rect.IsEmpty) { $null } else { [int]$rect.Y }
    w         = if ($rect.IsEmpty) { $null } else { [int]$rect.Width }
    h         = if ($rect.IsEmpty) { $null } else { [int]$rect.Height }
  }
}
$payload = [pscustomobject]@{ title = $window.Current.Name; count = $elements.Count; elements = @($rows) }
$payload | ConvertTo-Json -Depth 5 -Compress | Set-Content -Path $Out -Encoding UTF8
Write-Output "uia elements: $($elements.Count)"
