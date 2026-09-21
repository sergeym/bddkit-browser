Feature: every kind of form control

  Scenario: select, checkbox, textarea and a file input
    Given I am on "/profile"
    When I select "Latvia" from "Country"
    And I check "Newsletter"
    And I fill in "About you" with "Tester at large"
    Then the "country" field should contain "lv"
    And the "Newsletter" checkbox should be checked
    And the "bio" field should contain "Tester at large"
    When I uncheck "Newsletter"
    Then the "Newsletter" checkbox should be unchecked
    When I read the "placeholder" attribute of "#bio" as "hint"
    And I execute the script "return document.title"
    Then variable "hint" should be equal to "A line or two"
    And variable "script_result" should be equal to "Profile — bddkit demo"
    When I check "Newsletter"
    And I press "Save"
    Then the "[data-test=summary]" element should contain "country=lv newsletter=yes bio=Tester at large"
